//! Single items: song and video rows, album, playlist and podcast cards,
//! artist rows, episodes, mood tiles and the buttons to other pages.

use reader::models::{ArtistCredit, Track};
use serde_json::Value;

use super::{decode_percent, page_id, renderer, runs, text, thumbnail};
use crate::ytmusic::actions;
use crate::ytmusic::discover::{DiscoverItem, ItemActions, LinkKind, PageLink};
use crate::ytmusic::search::{ParsedRow, parsed_to_track};

/// Any item renderer, or `None` for kinds there is no use for.
pub(super) fn item(v: &Value) -> Option<DiscoverItem> {
    let (key, r) = renderer(v)?;
    let parsed = match key {
        "musicResponsiveListItemRenderer" => responsive_list_item(r),
        "musicTwoRowItemRenderer" => two_row_item(r),
        "musicMultiRowListItemRenderer" => multi_row_item(r),
        "musicNavigationButtonRenderer" => navigation_button(r),
        _ => {
            tracing::debug!(renderer = key, "skipping item renderer");
            return None;
        }
    };
    if parsed.is_none() {
        tracing::debug!(renderer = key, "item without the fields it needs");
    }
    parsed
}

pub(super) fn items(list: &Value) -> Vec<DiscoverItem> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(item)
        .collect()
}

/// What a link in a page leads to, told apart by its browse id.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Album(String),
    /// The channel id, also for a library row's `MPLA` link to the artist's
    /// songs in the library, which is the same artist.
    Artist(String),
    Playlist(String),
    Podcast(String),
    Episode(String),
    Other,
}

fn target(endpoint: &Value) -> Option<Target> {
    let browse = &endpoint["browseEndpoint"];
    let id = browse["browseId"].as_str()?;
    // An album's link carries params too, but only a channel's pick a view of it.
    let view = browse["params"].is_string();
    Some(match id {
        _ if id.starts_with("MPRE") => Target::Album(id.to_string()),
        _ if id.starts_with("MPSP") => Target::Podcast(id.to_string()),
        _ if id.starts_with("MPED") => Target::Episode(id.to_string()),
        _ if id.starts_with("MPLAUC") => Target::Artist(id["MPLA".len()..].to_string()),
        _ if id.starts_with("UC") && !view => Target::Artist(id.to_string()),
        _ if id.starts_with("VL") => Target::Playlist(id["VL".len()..].to_string()),
        _ => Target::Other,
    })
}

/// A run as a name that may link to a page.
#[derive(Debug, Clone)]
struct Link {
    text: String,
    target: Option<Target>,
}

fn link(run: &Value) -> Option<Link> {
    let text = run["text"].as_str()?.trim();
    if is_joiner(text) {
        return None;
    }
    Some(Link {
        text: text.to_string(),
        target: target(&run["navigationEndpoint"]),
    })
}

/// The ", " and " & " runs between artist names.
fn is_joiner(text: &str) -> bool {
    matches!(text.trim(), "," | "&" | "•" | "" | "and" | "·")
}

/// Runs split on the " • " separators YouTube puts between subtitle parts.
fn run_groups(runs: &[Value]) -> Vec<Vec<&Value>> {
    let mut groups = vec![Vec::new()];
    for run in runs {
        if run["text"].as_str().is_some_and(|t| t.trim() == "•") {
            groups.push(Vec::new());
        } else if let Some(last) = groups.last_mut() {
            last.push(run);
        }
    }
    groups.retain(|group| !group.is_empty());
    groups
}

const LABELS: [&str; 13] = [
    "Song",
    "Video",
    "Album",
    "Single",
    "EP",
    "Playlist",
    "Artist",
    "Podcast",
    "Episode",
    "Profile",
    "Station",
    "Audiobook",
    "Chart",
];

/// A subtitle or byline broken into what it names. The parts come in
/// different orders on different pages, so each `•` group is classified by
/// its links and its shape rather than by its position.
#[derive(Debug, Default)]
struct Byline {
    label: Option<String>,
    artists: Vec<Link>,
    album: Option<Link>,
    duration_secs: Option<u64>,
    plays: Option<String>,
    /// When an episode came out, as the page wrote it: "Feb 25, 2024", "10h ago".
    published: Option<String>,
    /// Groups that matched nothing above, in order.
    rest: Vec<String>,
}

impl Byline {
    /// Each column is classified on its own, since a column break separates
    /// parts the way " • " does.
    fn from_columns<'a>(columns: impl IntoIterator<Item = &'a [Value]>) -> Self {
        let mut byline = Self::default();
        for runs in columns {
            for group in run_groups(runs) {
                byline.add(&group);
            }
        }
        byline
    }

    fn add(&mut self, group: &[&Value]) {
        let text: String = group
            .iter()
            .filter_map(|run| run["text"].as_str())
            .collect::<String>()
            .trim()
            .to_string();
        if text.is_empty() {
            return;
        }
        let targets: Vec<Target> = group
            .iter()
            .filter_map(|run| target(&run["navigationEndpoint"]))
            .collect();
        let is_album = |t: &Target| matches!(t, Target::Album(_));
        if self.album.is_none() && targets.iter().any(is_album) {
            self.album = group
                .iter()
                .find_map(|run| link(run).filter(|l| l.target.as_ref().is_some_and(is_album)));
        } else if self.artists.is_empty() && targets.iter().any(|t| matches!(t, Target::Artist(_)))
        {
            self.artists = group.iter().filter_map(|run| link(run)).collect();
        } else if let Some(secs) = parse_clock(&text)
            .or_else(|| parse_spoken_duration(&text))
            .filter(|_| self.duration_secs.is_none())
        {
            self.duration_secs = Some(secs);
        } else if self.plays.is_none() && is_count(&text) {
            self.plays = Some(text);
        } else if self.published.is_none() && is_date(&text) {
            self.published = Some(text);
        } else if self.label.is_none() && self.is_first_group() && LABELS.contains(&text.as_str()) {
            self.label = Some(text);
        } else {
            self.rest.push(text);
        }
    }

    fn is_first_group(&self) -> bool {
        self.artists.is_empty() && self.album.is_none() && self.rest.is_empty()
    }

    /// Linked artists, or the first unlinked group, kept whole, when the page
    /// left names unlinked.
    fn credits(&self) -> Vec<ArtistCredit> {
        if self.artists.is_empty() {
            return self
                .rest
                .first()
                .map(|name| vec![ArtistCredit::unlinked(name.as_str())])
                .unwrap_or_default();
        }
        self.artists
            .iter()
            .map(|artist| match &artist.target {
                Some(Target::Artist(id)) => ArtistCredit::linked(artist.text.as_str(), id),
                _ => ArtistCredit::unlinked(artist.text.as_str()),
            })
            .collect()
    }
}

/// "900M views", "1.2M subscribers"; a show called "Full Interviews" is a name.
fn is_count(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_digit())
        && [
            "views",
            "plays",
            "view",
            "play",
            "listeners",
            "subscribers",
            "monthly audience",
        ]
        .iter()
        .any(|suffix| text.ends_with(suffix))
}

/// "4:36", "1:02:03" in seconds.
fn parse_clock(text: &str) -> Option<u64> {
    let text = text.trim();
    if !text.contains(':') {
        return None;
    }
    text.split(':')
        .try_fold(0u64, |acc, part| Some(acc * 60 + part.parse::<u64>().ok()?))
}

/// "1 hr 3 min", "45 min", "6 min 18 sec" from podcast episodes, in
/// seconds. The whole text has to be a duration, since a byline group that
/// only mentions minutes is something else.
fn parse_spoken_duration(text: &str) -> Option<u64> {
    let mut total = 0;
    let mut found = false;
    let mut words = text.split_whitespace();
    while let Some(word) = words.next() {
        let n = word.parse::<u64>().ok()?;
        let secs = match words.next()?.trim_end_matches(['s', ',']) {
            "hr" | "hour" => 3600,
            "min" | "minute" => 60,
            "sec" | "second" => 1,
            _ => return None,
        };
        total += n * secs;
        found = true;
    }
    found.then_some(total)
}

/// "Feb 25, 2024", "Jul 21", "10h ago", "3 days ago": an episode's release.
fn is_date(text: &str) -> bool {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    if text.ends_with(" ago") {
        return true;
    }
    let mut words = text.split_whitespace();
    let (Some(month), Some(day)) = (words.next(), words.next()) else {
        return false;
    };
    MONTHS.contains(&month)
        && day.trim_end_matches(',').parse::<u8>().is_ok()
        && words.all(|year| year.parse::<u16>().is_ok())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Song,
    Video,
    Episode,
}

fn video_kind(watch: &Value) -> Option<Kind> {
    let kind =
        watch["watchEndpointMusicSupportedConfigs"]["watchEndpointMusicConfig"]["musicVideoType"]
            .as_str()?;
    Some(match kind {
        "MUSIC_VIDEO_TYPE_ATV" | "MUSIC_VIDEO_TYPE_PRIVATELY_OWNED_TRACK" => Kind::Song,
        "MUSIC_VIDEO_TYPE_PODCAST_EPISODE" => Kind::Episode,
        _ => Kind::Video,
    })
}

fn kind_from_label(label: Option<&str>) -> Option<Kind> {
    match label? {
        "Song" => Some(Kind::Song),
        "Video" => Some(Kind::Video),
        "Episode" => Some(Kind::Episode),
        _ => None,
    }
}

/// A row that plays, as the track the library would build for it: the same
/// row type and the same album id the library sync derives.
fn track(video_id: &str, title: String, byline: &Byline, thumb: Option<String>) -> Track {
    let album = byline.album.as_ref();
    parsed_to_track(ParsedRow {
        video_id: video_id.to_string(),
        title,
        artists: byline.credits(),
        album: album.map(|album| album.text.clone()),
        album_browse_id: album.and_then(|album| match &album.target {
            Some(Target::Album(id)) => Some(id.clone()),
            _ => None,
        }),
        duration: byline.duration_secs.unwrap_or(0),
        thumbnail_url: thumb,
    })
}

fn playable(
    kind: Kind,
    track: Track,
    actions: ItemActions,
    published: Option<String>,
) -> DiscoverItem {
    match kind {
        Kind::Song => DiscoverItem::Song(Box::new(track), actions),
        Kind::Video => DiscoverItem::Video(Box::new(track), actions),
        Kind::Episode => {
            let browse_id = format!("MPED{}", track.id.key());
            DiscoverItem::Episode {
                track: Box::new(track),
                browse_id,
                published,
            }
        }
    }
}

/// `musicResponsiveListItemRenderer`: every row-shaped item, from result
/// rows and library songs to library artists.
pub(super) fn responsive_list_item(r: &Value) -> Option<DiscoverItem> {
    let columns: Vec<&Value> = r["flexColumns"]
        .as_array()?
        .iter()
        .map(|c| &c["musicResponsiveListItemFlexColumnRenderer"]["text"])
        .collect();
    let title_column = columns.first()?;
    let title = text(title_column)?;
    let title_endpoint = runs(title_column)
        .first()
        .map(|run| &run["navigationEndpoint"])
        .unwrap_or(&Value::Null);

    let fixed = r["fixedColumns"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| runs(&c["musicResponsiveListItemFixedColumnRenderer"]["text"]));
    let byline = Byline::from_columns(columns[1..].iter().map(|c| runs(c)).chain(fixed));
    let subtitle: Vec<String> = columns[1..].iter().filter_map(|c| text(c)).collect();
    let subtitle = subtitle.join(" • ");
    let thumb = thumbnail(&r["thumbnail"]);

    let overlay = &r["overlay"]["musicItemThumbnailOverlayRenderer"]["content"]["musicPlayButtonRenderer"]
        ["playNavigationEndpoint"]["watchEndpoint"];
    let watch = match &title_endpoint["watchEndpoint"] {
        watch if watch.is_object() => watch,
        _ => overlay,
    };
    let video_id = r["playlistItemData"]["videoId"]
        .as_str()
        .or_else(|| watch["videoId"].as_str());
    let menu = &r["menu"];

    match (
        target(&r["navigationEndpoint"]).or_else(|| target(title_endpoint)),
        video_id,
    ) {
        (Some(Target::Album(browse_id)), _) => Some(DiscoverItem::Album {
            actions: actions::playlist(menu, actions::overlay_playlist_id(&r["overlay"])),
            browse_id,
            title,
            subtitle,
            thumbnail: thumb,
        }),
        (Some(Target::Artist(channel_id)), _) => Some(DiscoverItem::Artist {
            actions: actions::artist(&channel_id),
            channel_id,
            name: title,
            subtitle: (!subtitle.is_empty()).then_some(subtitle),
            thumbnail: thumb,
        }),
        (Some(Target::Playlist(playlist_id)), _) => Some(DiscoverItem::Playlist {
            actions: actions::playlist(menu, Some(&playlist_id)),
            playlist_id,
            title,
            subtitle,
            thumbnail: thumb,
        }),
        (Some(Target::Podcast(browse_id)), _) => Some(DiscoverItem::Podcast {
            browse_id,
            title,
            subtitle,
            thumbnail: thumb,
        }),
        (Some(Target::Episode(browse_id)), video_id) => {
            let video_id = video_id.or_else(|| browse_id.strip_prefix("MPED"))?;
            Some(DiscoverItem::Episode {
                track: Box::new(track(video_id, title, &byline, thumb)),
                browse_id,
                published: byline.published,
            })
        }
        (_, Some(video_id)) => {
            let kind = video_kind(watch)
                .or_else(|| kind_from_label(byline.label.as_deref()))
                .unwrap_or(Kind::Song);
            let actions = actions::track(menu, video_id);
            let track = track(video_id, title, &byline, thumb);
            Some(playable(kind, track, actions, byline.published))
        }
        (_, None) => None,
    }
}

/// `musicTwoRowItemRenderer`: the square cards in carousels and grids.
pub(super) fn two_row_item(r: &Value) -> Option<DiscoverItem> {
    let title = text(&r["title"])?;
    let endpoint = &r["navigationEndpoint"];
    let byline = Byline::from_columns([runs(&r["subtitle"])]);
    let subtitle = text(&r["subtitle"]).unwrap_or_default();
    let thumb = thumbnail(&r["thumbnailRenderer"]);
    let menu = &r["menu"];

    if let Some(video_id) = endpoint["watchEndpoint"]["videoId"].as_str() {
        let kind = video_kind(&endpoint["watchEndpoint"]).unwrap_or(Kind::Video);
        let actions = actions::track(menu, video_id);
        let track = track(video_id, title, &byline, thumb);
        return Some(playable(kind, track, actions, byline.published));
    }
    if let Some(playlist_id) = endpoint["watchPlaylistEndpoint"]["playlistId"].as_str() {
        return Some(DiscoverItem::Playlist {
            playlist_id: playlist_id.to_string(),
            title,
            subtitle,
            thumbnail: thumb,
            actions: actions::playlist(menu, Some(playlist_id)),
        });
    }
    match target(endpoint)? {
        Target::Album(browse_id) => Some(DiscoverItem::Album {
            actions: actions::playlist(menu, actions::overlay_playlist_id(&r["thumbnailOverlay"])),
            browse_id,
            title,
            subtitle,
            thumbnail: thumb,
        }),
        Target::Artist(channel_id) => Some(DiscoverItem::Artist {
            actions: actions::artist(&channel_id),
            channel_id,
            name: title,
            subtitle: (!subtitle.is_empty()).then_some(subtitle),
            thumbnail: thumb,
        }),
        Target::Playlist(playlist_id) => Some(DiscoverItem::Playlist {
            actions: actions::playlist(menu, Some(&playlist_id)),
            playlist_id,
            title,
            subtitle,
            thumbnail: thumb,
        }),
        Target::Podcast(browse_id) => Some(DiscoverItem::Podcast {
            browse_id,
            title,
            subtitle,
            thumbnail: thumb,
        }),
        Target::Episode(browse_id) => {
            let video_id = browse_id.strip_prefix("MPED")?.to_string();
            Some(DiscoverItem::Episode {
                track: Box::new(track(&video_id, title, &byline, thumb)),
                browse_id,
                published: byline.published,
            })
        }
        Target::Other => {
            let link = PageLink::endpoint(endpoint)?;
            (link.kind == LinkKind::Page).then_some(DiscoverItem::Page {
                page_id: link.id,
                title,
            })
        }
    }
}

/// `musicMultiRowListItemRenderer`: a podcast episode, with its description
/// under the title.
fn multi_row_item(r: &Value) -> Option<DiscoverItem> {
    let video_id = r["onTap"]["watchEndpoint"]["videoId"].as_str()?;
    let progress = &r["playbackProgress"]["musicPlaybackProgressRenderer"];
    // The subtitle is "164K views • 10h ago"; the length sits in the
    // progress bar as " • 1 hr 3 min".
    let mut byline = Byline::from_columns([runs(&r["subtitle"])]);
    byline.duration_secs = text(&progress["durationText"])
        .as_deref()
        .and_then(|t| parse_spoken_duration(t.trim_start_matches([' ', '•'])))
        .or(byline.duration_secs);
    let track = track(
        video_id,
        text(&r["title"])?,
        &byline,
        thumbnail(&r["thumbnail"]),
    );
    Some(playable(
        Kind::Episode,
        track,
        ItemActions::default(),
        byline.published,
    ))
}

/// `musicNavigationButtonRenderer`: mood and genre tiles, and Explore's own
/// buttons to New releases, Charts and Moods & genres.
fn navigation_button(r: &Value) -> Option<DiscoverItem> {
    let browse = &r["clickCommand"]["browseEndpoint"];
    let browse_id = browse["browseId"].as_str()?;
    let params = browse["params"].as_str().map(decode_percent);
    let title = text(&r["buttonText"])?;
    let page_id = page_id(browse_id, params.as_deref());
    if browse_id == super::MOOD_CATEGORY {
        return Some(DiscoverItem::Mood {
            browse_id: page_id,
            title,
            thumbnail: None,
            accent: r["solid"]["leftStripeColor"].as_u64().map(|c| c as u32),
        });
    }
    Some(DiscoverItem::Page { page_id, title })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_byline_is_read_by_what_each_part_is() {
        let runs = json!([
            {"text": "Song"}, {"text": " • "},
            {"text": "Daft Punk", "navigationEndpoint": {"browseEndpoint": {"browseId": "UCabc"}}},
            {"text": " & "},
            {"text": "Julian Casablancas", "navigationEndpoint": {"browseEndpoint": {"browseId": "UCdef"}}},
            {"text": " • "},
            {"text": "Random Access Memories", "navigationEndpoint": {"browseEndpoint": {"browseId": "MPREb_x"}}},
            {"text": " • "}, {"text": "5:38"}
        ]);
        let byline = Byline::from_columns([runs.as_array().unwrap().as_slice()]);
        assert_eq!(byline.label.as_deref(), Some("Song"));
        let credits = byline.credits();
        assert_eq!(credits.len(), 2);
        assert_eq!(credits[1].id.as_deref(), Some("UCdef"));
        assert_eq!(
            byline.album.map(|album| album.text).as_deref(),
            Some("Random Access Memories")
        );
        assert_eq!(byline.duration_secs, Some(338));
    }

    /// Unlinked text is one credit exactly as written: cutting it guesses at names.
    #[test]
    fn an_unlinked_artist_is_kept_whole() {
        let runs = json!([{"text": "Ada & Boris"}, {"text": " • "}, {"text": "900M views"}]);
        let byline = Byline::from_columns([runs.as_array().unwrap().as_slice()]);
        let credits = byline.credits();
        assert_eq!(credits.len(), 1);
        assert_eq!(credits[0].name, "Ada & Boris");
        assert_eq!(byline.plays.as_deref(), Some("900M views"));
    }

    #[test]
    fn durations_read_both_ways_youtube_writes_them() {
        assert_eq!(parse_clock("4:36"), Some(276));
        assert_eq!(parse_clock("1:02:03"), Some(3723));
        assert_eq!(parse_clock("2013"), None);
        assert_eq!(parse_spoken_duration("1 hr 3 min"), Some(3780));
        assert_eq!(parse_spoken_duration("45 min"), Some(2700));
        assert_eq!(parse_spoken_duration("6 min 18 sec"), Some(378));
        assert_eq!(parse_spoken_duration("Played"), None);
        assert_eq!(parse_spoken_duration("3 min read"), None);
        assert_eq!(parse_spoken_duration("Top 50 songs"), None);
    }

    #[test]
    fn release_dates_are_told_from_names() {
        assert!(is_date("Feb 25, 2024"));
        assert!(is_date("Jul 21"));
        assert!(is_date("10h ago"));
        assert!(is_date("3 days ago"));
        assert!(!is_date("May Erlewine"));
        assert!(!is_date("Entertainment of Excellence"));
    }

    /// A library artist row links the artist's songs in the library, which
    /// opens as the same artist.
    #[test]
    fn a_library_artist_link_is_the_channel() {
        let endpoint = json!({"browseEndpoint": {"browseId": "MPLAUCabc"}});
        assert_eq!(target(&endpoint), Some(Target::Artist("UCabc".into())));
    }
}

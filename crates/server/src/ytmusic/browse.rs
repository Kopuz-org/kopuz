//! Every browse page beyond the ones `discover` reads by hand: Explore,
//! charts, moods and genres, new releases, podcasts, the library tabs,
//! history, an artist's "show all" pages and a song's Related tab.
//!
//! They are all built from the same renderers, so one parser reads them as
//! the web app lays them out: a header, the chips over the page, and shelves.
//! Fields are read by index, which yields `Null` for anything missing, so a
//! renamed field degrades to an absent value rather than a lost page.

mod items;
pub mod search;
mod shelves;

use serde_json::{Value, json};

use super::clients::WEB_REMIX;
use super::discover::{BrowsePage, DiscoverShelf, PageChip, ShelfLayout};
use super::innertube;

pub const EXPLORE: &str = "FEmusic_explore";
pub const NEW_RELEASES: &str = "FEmusic_new_releases";
pub const CHARTS: &str = "FEmusic_charts";
pub const MOODS: &str = "FEmusic_moods_and_genres";
pub const MOOD_CATEGORY: &str = "FEmusic_moods_and_genres_category";
/// YouTube Music has no podcasts page of its own any more, only the Podcasts
/// chip over Home; these params select that chip alone.
pub const PODCASTS: &str = "FEmusic_home?ggMGSgQIDBAD";
pub const HISTORY: &str = "FEmusic_history";
pub const LIBRARY_SONGS: &str = "FEmusic_liked_videos";
pub const LIBRARY_ALBUMS: &str = "FEmusic_liked_albums";
pub const LIBRARY_ARTISTS: &str = "FEmusic_library_corpus_track_artists";
pub const LIBRARY_SUBSCRIPTIONS: &str = "FEmusic_library_corpus_artists";
pub const LIBRARY_PODCASTS: &str = "FEmusic_library_non_music_audio_list";
pub const LIBRARY_UPLOADS: &str = "FEmusic_library_privately_owned_tracks";

/// A page's id: its browse id, and the `params` that pick a view of it,
/// joined by `?`. Neither half ever contains one.
pub fn page_id(browse_id: &str, params: Option<&str>) -> String {
    match params {
        Some(params) => format!("{browse_id}?{params}"),
        None => browse_id.to_string(),
    }
}

pub fn split_page_id(id: &str) -> (&str, Option<&str>) {
    match id.split_once('?') {
        Some((browse_id, params)) => (browse_id, Some(params)),
        None => (id, None),
    }
}

/// Params arrive URL-encoded inside JSON (`...%3D`) and are sent back decoded.
pub fn decode_percent(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[tracing::instrument(name = "yt.browse_page", skip(cookies))]
pub async fn fetch_page(id: &str, cookies: Option<&str>) -> Result<BrowsePage, String> {
    let (browse_id, params) = split_page_id(id);
    let mut payload = json!({ "browseId": browse_id });
    if let Some(params) = params {
        payload["params"] = params.into();
    }
    let response = innertube::post(WEB_REMIX, "browse", payload, cookies).await?;
    let mut page =
        parse_page(&response).ok_or_else(|| format!("{browse_id} is not a browse page"))?;
    let visitor = innertube::extract_visitor_data(&response);
    tag_continuations(&mut page, Endpoint::Browse, visitor.as_deref());
    Ok(page)
}

/// More of a page, or more of one shelf, from a token either handed out.
#[tracing::instrument(name = "yt.browse_continuation", skip_all)]
pub async fn fetch_continuation(token: &str, cookies: Option<&str>) -> Result<BrowsePage, String> {
    let continuation = Continuation::read(token);
    let response = innertube::post_as_visitor(
        WEB_REMIX,
        continuation.endpoint.path(),
        json!({ "continuation": continuation.token }),
        cookies,
        continuation.visitor,
    )
    .await?;
    let mut page = parse_continuation(&response);
    stop_repeats(continuation.token, &mut page);
    tag_continuations(&mut page, continuation.endpoint, continuation.visitor);
    Ok(page)
}

/// Which endpoint a continuation goes back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Endpoint {
    Browse,
    Search,
}

impl Endpoint {
    fn path(self) -> &'static str {
        match self {
            Self::Browse => "browse",
            Self::Search => "search",
        }
    }
}

/// A continuation as this module hands it out: YouTube's token, the endpoint
/// it goes back to and the visitor it was issued to, since an anonymous
/// token answers nobody else. A bare token, as `discover` hands out, is a
/// browse token for no one in particular.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Continuation<'a> {
    pub endpoint: Endpoint,
    pub visitor: Option<&'a str>,
    pub token: &'a str,
}

impl<'a> Continuation<'a> {
    const SEPARATOR: char = '|';

    pub fn write(endpoint: Endpoint, visitor: Option<&str>, token: &str) -> String {
        let sep = Self::SEPARATOR;
        format!(
            "{}{sep}{}{sep}{token}",
            endpoint.path(),
            visitor.unwrap_or_default()
        )
    }

    pub fn read(handed_out: &'a str) -> Self {
        let mut parts = handed_out.splitn(3, Self::SEPARATOR);
        let (Some(endpoint), Some(visitor), Some(token)) =
            (parts.next(), parts.next(), parts.next())
        else {
            return Self {
                endpoint: Endpoint::Browse,
                visitor: None,
                token: handed_out,
            };
        };
        Self {
            endpoint: match endpoint {
                "search" => Endpoint::Search,
                _ => Endpoint::Browse,
            },
            visitor: (!visitor.is_empty()).then_some(visitor),
            token,
        }
    }
}

pub(crate) fn tag_continuations(page: &mut BrowsePage, endpoint: Endpoint, visitor: Option<&str>) {
    let tag = |token: &mut Option<String>| {
        if let Some(raw) = token.take() {
            *token = Some(Continuation::write(endpoint, visitor, &raw));
        }
    };
    tag(&mut page.continuation);
    page.shelves
        .iter_mut()
        .for_each(|shelf| tag(&mut shelf.continuation));
}

/// Search, all of it or under one filter, or more of a filtered search.
#[tracing::instrument(name = "yt.search_page", skip(cookies))]
pub async fn fetch_search(
    query: &str,
    filter: Option<&'static search::Filter>,
    cookies: Option<&str>,
) -> Result<search::SearchPage, String> {
    let mut payload = json!({ "query": query });
    if let Some(filter) = filter {
        payload["params"] = filter.params().into();
    }
    let response = innertube::post(WEB_REMIX, "search", payload, cookies).await?;
    let mut page = search::parse_search(filter, &response);
    let visitor = innertube::extract_visitor_data(&response);
    if let Some(token) = page.continuation.take() {
        page.continuation = Some(Continuation::write(
            Endpoint::Search,
            visitor.as_deref(),
            &token,
        ));
    }
    Ok(page)
}

#[tracing::instrument(name = "yt.search_suggestions", skip(cookies))]
pub async fn fetch_suggestions(
    query: &str,
    cookies: Option<&str>,
) -> Result<Vec<search::Suggestion>, String> {
    let response = innertube::post(
        WEB_REMIX,
        "music/get_search_suggestions",
        json!({ "input": query }),
        cookies,
    )
    .await?;
    Ok(search::parse_suggestions(&response))
}

/// A song's Related tab: the watch page names it, and it browses like any page.
#[tracing::instrument(name = "yt.related", skip(cookies))]
pub async fn fetch_related(video_id: &str, cookies: Option<&str>) -> Result<BrowsePage, String> {
    let watch = innertube::post(
        WEB_REMIX,
        "next",
        json!({ "videoId": video_id, "isAudioOnly": true }),
        cookies,
    )
    .await?;
    match related_browse_id(&watch) {
        Some(browse_id) => fetch_page(&browse_id, cookies).await,
        None => Ok(BrowsePage::default()),
    }
}

pub(super) fn related_browse_id(watch: &Value) -> Option<String> {
    watch["contents"]["singleColumnMusicWatchNextResultsRenderer"]["tabbedRenderer"]
        ["watchNextTabbedResultsRenderer"]["tabs"]
        .as_array()?
        .iter()
        .filter_map(|tab| tab["tabRenderer"]["endpoint"]["browseEndpoint"]["browseId"].as_str())
        .find(|id| id.starts_with("MPTR"))
        .map(str::to_string)
}

/// YouTube sometimes answers a continuation with the token it was sent, or
/// with a token and nothing before it; following either never ends.
fn stop_repeats(token: &str, page: &mut BrowsePage) {
    let empty = page.shelves.iter().all(|shelf| shelf.items.is_empty());
    if empty || page.continuation.as_deref() == Some(token) {
        page.continuation = None;
    }
    for shelf in &mut page.shelves {
        if shelf.items.is_empty() || shelf.continuation.as_deref() == Some(token) {
            shelf.continuation = None;
        }
    }
}

/// A whole `browse` response: the single-column layout (Home, Explore, the
/// library), the two-column one (playlists, podcasts, episodes), or a bare
/// section list. `None` when none of them is there.
pub(crate) fn parse_page(response: &Value) -> Option<BrowsePage> {
    let contents = &response["contents"];
    let mut header = shelves::header(&response["header"]);
    let mut lead = Vec::new();

    let list = if let Some(tabs) = contents["singleColumnBrowseResultsRenderer"]["tabs"].as_array()
    {
        let tab = tabs
            .iter()
            .find(|t| t["tabRenderer"]["selected"].as_bool() == Some(true))
            .or(tabs.first())?;
        &tab["tabRenderer"]["content"]["sectionListRenderer"]
    } else if contents["twoColumnBrowseResultsRenderer"].is_object() {
        let two = &contents["twoColumnBrowseResultsRenderer"];
        let primary = &two["tabs"][0]["tabRenderer"]["content"]["sectionListRenderer"]["contents"];
        for entry in primary.as_array().into_iter().flatten() {
            if let Some(found) = shelves::header(entry) {
                header = Some(found);
            } else if let Some((key, renderer)) = renderer(entry) {
                lead.extend(shelves::section(key, renderer));
            }
        }
        if header.is_none() && lead.is_empty() {
            return None;
        }
        &two["secondaryContents"]["sectionListRenderer"]
    } else if contents["sectionListRenderer"].is_object() {
        &contents["sectionListRenderer"]
    } else {
        return None;
    };

    let about = list["contents"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|entry| text(&entry["musicDescriptionShelfRenderer"]["description"]));
    let mut found = lead;
    found.extend(shelves::sections(&list["contents"]));

    let mut page = headed(header.unwrap_or_default());
    page.description = page.description.or(about);
    page.chips = chips(&list["header"]["chipCloudRenderer"]);
    page.shelves = found;
    page.continuation = continuation(list);
    Some(page)
}

fn headed(header: shelves::Header) -> BrowsePage {
    BrowsePage {
        header: header.kind,
        title: header.title,
        subtitle: header.subtitle,
        description: header.description,
        thumbnail: header.thumbnail,
        playback_id: header.playback_id,
        actions: header.actions,
        privacy: header.privacy,
        ..BrowsePage::default()
    }
}

/// Only the header of a page, as a page with nothing else filled in: what a
/// playlist's first page says about the playlist beside its tracks.
pub(crate) fn page_header(response: &Value) -> Option<BrowsePage> {
    let primary = &response["contents"]["twoColumnBrowseResultsRenderer"]["tabs"][0]["tabRenderer"]
        ["content"]["sectionListRenderer"]["contents"];
    shelves::header(&response["header"])
        .or_else(|| {
            primary
                .as_array()
                .into_iter()
                .flatten()
                .find_map(shelves::header)
        })
        .map(headed)
}

/// A continuation answer. More sections continue the page, and come back as
/// its shelves with the page's next token; more items continue one shelf,
/// and come back as a single shelf carrying that shelf's next token.
pub(crate) fn parse_continuation(response: &Value) -> BrowsePage {
    if let Some((key, body)) = response["continuationContents"]
        .as_object()
        .and_then(|contents| contents.iter().next())
    {
        if key == "sectionListContinuation" {
            return BrowsePage {
                shelves: shelves::sections(&body["contents"]),
                continuation: continuation(body),
                ..BrowsePage::default()
            };
        }
        let list = if body["items"].is_array() {
            &body["items"]
        } else {
            &body["contents"]
        };
        let layout = match key.as_str() {
            "gridContinuation" => ShelfLayout::Grid,
            _ => ShelfLayout::List,
        };
        return more_items(items::items(list), layout, continuation(body));
    }

    let mut page = BrowsePage::default();
    let mut found = Vec::new();
    let mut next = None;
    for action in response["onResponseReceivedActions"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let list = &action["appendContinuationItemsAction"]["continuationItems"];
        let list = if list.is_array() {
            list
        } else {
            &action["reloadContinuationItemsCommand"]["continuationItems"]
        };
        for entry in list.as_array().into_iter().flatten() {
            let Some((key, renderer)) = renderer(entry) else {
                continue;
            };
            if key == "continuationItemRenderer" {
                next = continuation_item(entry);
            } else if let Some(item) = items::item(entry) {
                found.push(item);
            } else if let Some(shelf) = shelves::section(key, renderer) {
                page.shelves.push(shelf);
            }
        }
    }
    if !found.is_empty() {
        return more_items(found, ShelfLayout::List, next);
    }
    page.continuation = next;
    page
}

fn more_items(
    items: Vec<super::discover::DiscoverItem>,
    layout: ShelfLayout,
    next: Option<String>,
) -> BrowsePage {
    BrowsePage {
        shelves: vec![DiscoverShelf {
            title: String::new(),
            strapline: None,
            more: None,
            items,
            layout,
            continuation: next,
            search_filter: None,
        }],
        ..BrowsePage::default()
    }
}

/// The chips over a page, each opening the page it filters to.
fn chips(cloud: &Value) -> Vec<PageChip> {
    cloud["chips"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|chip| {
            let chip = &chip["chipCloudChipRenderer"];
            let browse = &chip["navigationEndpoint"]["browseEndpoint"];
            let browse_id = browse["browseId"].as_str()?;
            let params = browse["params"].as_str().map(decode_percent);
            Some(PageChip {
                title: text(&chip["text"])?,
                page_id: page_id(browse_id, params.as_deref()),
                selected: chip["isSelected"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

/// The text of a `{runs: [...]}` or `{simpleText}` object, `None` when blank.
fn text(v: &Value) -> Option<String> {
    let joined = match v["simpleText"].as_str() {
        Some(simple) => simple.to_string(),
        None => v["runs"]
            .as_array()?
            .iter()
            .filter_map(|run| run["text"].as_str())
            .collect(),
    };
    (!joined.trim().is_empty()).then_some(joined)
}

fn runs(v: &Value) -> &[Value] {
    v["runs"].as_array().map(Vec::as_slice).unwrap_or_default()
}

/// The largest thumbnail under any of the wrappers YouTube nests them in.
fn thumbnail(v: &Value) -> Option<String> {
    [
        &v["thumbnails"],
        &v["thumbnail"]["thumbnails"],
        &v["musicThumbnailRenderer"]["thumbnail"]["thumbnails"],
        &v["thumbnail"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"],
        &v["croppedSquareThumbnailRenderer"]["thumbnail"]["thumbnails"],
        &v["thumbnail"]["croppedSquareThumbnailRenderer"]["thumbnail"]["thumbnails"],
        &v["thumbnailRenderer"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"],
        &v["thumbnailRenderer"]["croppedSquareThumbnailRenderer"]["thumbnail"]["thumbnails"],
    ]
    .into_iter()
    .find_map(Value::as_array)?
    .iter()
    .max_by_key(|t| t["width"].as_u64().unwrap_or(0))
    .and_then(|t| t["url"].as_str())
    .map(|url| match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_string(),
    })
    .map(super::discover::normalize_yt_thumbnail)
}

/// The token of a `continuations` entry, or of a trailing `continuationItemRenderer`.
fn continuation(v: &Value) -> Option<String> {
    v["continuations"]
        .as_array()
        .and_then(|list| {
            list.iter().find_map(|entry| {
                entry
                    .as_object()?
                    .values()
                    .find_map(|data| data["continuation"].as_str())
                    .map(str::to_string)
            })
        })
        .or_else(|| continuation_item(v["contents"].as_array()?.last()?))
}

fn continuation_item(item: &Value) -> Option<String> {
    let renderer = &item["continuationItemRenderer"];
    renderer["continuationEndpoint"]["continuationCommand"]["token"]
        .as_str()
        .or_else(|| {
            renderer["button"]["buttonRenderer"]["command"]["continuationCommand"]["token"].as_str()
        })
        .map(str::to_string)
}

/// The single key of a `{"somethingRenderer": {...}}` wrapper.
fn renderer(v: &Value) -> Option<(&str, &Value)> {
    v.as_object()?
        .iter()
        .find(|(key, _)| key.ends_with("Renderer") || key.ends_with("Model"))
        .map(|(key, value)| (key.as_str(), value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ytmusic::discover::DiscoverItem;

    #[test]
    fn a_continuation_keeps_its_endpoint_and_visitor() {
        let written = Continuation::write(Endpoint::Search, Some("CgtWaXNpdG9y%3D%3D"), "4qmFsgK");
        assert_eq!(
            Continuation::read(&written),
            Continuation {
                endpoint: Endpoint::Search,
                visitor: Some("CgtWaXNpdG9y%3D%3D"),
                token: "4qmFsgK",
            }
        );
        let signed_in = Continuation::write(Endpoint::Browse, None, "4qmFsgK");
        assert_eq!(Continuation::read(&signed_in).visitor, None);
        // A token `discover` handed out carries nothing else.
        assert_eq!(
            Continuation::read("4qmFsgK"),
            Continuation {
                endpoint: Endpoint::Browse,
                visitor: None,
                token: "4qmFsgK",
            }
        );
    }

    #[test]
    fn a_repeated_or_empty_continuation_ends_the_walk() {
        let shelf = |items: Vec<DiscoverItem>, next: &str| DiscoverShelf {
            title: String::new(),
            strapline: None,
            more: None,
            items,
            layout: ShelfLayout::List,
            continuation: Some(next.to_string()),
            search_filter: None,
        };
        let page_item = DiscoverItem::Page {
            page_id: "FEmusic_charts".into(),
            title: "Charts".into(),
        };
        let mut page = BrowsePage {
            shelves: vec![
                shelf(vec![page_item.clone()], "same"),
                shelf(vec![], "other"),
            ],
            continuation: Some("same".into()),
            ..BrowsePage::default()
        };
        stop_repeats("same", &mut page);
        assert_eq!(page.continuation, None);
        assert_eq!(page.shelves[0].continuation, None);
        assert_eq!(page.shelves[1].continuation, None, "nothing came with it");

        let mut fresh = BrowsePage {
            shelves: vec![shelf(vec![page_item], "next")],
            continuation: Some("next".into()),
            ..BrowsePage::default()
        };
        stop_repeats("same", &mut fresh);
        assert_eq!(fresh.continuation.as_deref(), Some("next"));
        assert_eq!(fresh.shelves[0].continuation.as_deref(), Some("next"));
    }
}

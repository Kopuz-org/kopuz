use api::{ApiError, ErrorCode, ExternalPlaylist, TrackInfo};

const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:140.0) Gecko/20100101 Firefox/140.0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Song {
    pub artist: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongList {
    pub name: String,
    pub songs: Vec<Song>,
}

pub async fn read(source: ExternalPlaylist) -> Result<SongList, ApiError> {
    let list = match source {
        ExternalPlaylist::Link(link) => spotify(&link).await?,
        ExternalPlaylist::Csv { name, text } => SongList {
            name,
            songs: csv(&text)?,
        },
    };
    if list.songs.is_empty() {
        return Err(ApiError::invalid_input("the playlist lists no songs"));
    }
    Ok(list)
}

fn spotify_target(link: &str) -> Option<(&'static str, String)> {
    let url = reqwest::Url::parse(link.trim()).ok()?;
    if url.host_str()? != "open.spotify.com" {
        return None;
    }
    let mut parts = url
        .path_segments()?
        .filter(|part| !part.is_empty() && !part.starts_with("intl-") && *part != "embed");
    let kind = match parts.next()? {
        "playlist" => "playlist",
        "album" => "album",
        _ => return None,
    };
    let id = parts.next()?;
    id.chars()
        .all(|c| c.is_ascii_alphanumeric())
        .then(|| (kind, id.to_string()))
}

async fn spotify(link: &str) -> Result<SongList, ApiError> {
    let (kind, id) = spotify_target(link)
        .ok_or_else(|| ApiError::invalid_input("not a Spotify playlist or album link"))?;
    let page = utils::http::client()
        .get(format!("https://open.spotify.com/embed/{kind}/{id}"))
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|error| ApiError::new(ErrorCode::SourceUnreachable, error.to_string()))?
        .text()
        .await
        .map_err(|error| ApiError::new(ErrorCode::SourceUnreachable, error.to_string()))?;
    embed_songs(&page)
        .ok_or_else(|| ApiError::not_found("Spotify did not show this playlist; is it public?"))
}

fn embed_songs(page: &str) -> Option<SongList> {
    const OPEN: &str = r#"<script id="__NEXT_DATA__" type="application/json">"#;
    let start = page.find(OPEN)? + OPEN.len();
    let end = start + page[start..].find("</script>")?;
    let data: serde_json::Value = serde_json::from_str(&page[start..end]).ok()?;
    let entity = data.pointer("/props/pageProps/state/data/entity")?;
    let name = entity
        .get("name")
        .or_else(|| entity.get("title"))
        .and_then(|name| name.as_str())
        .unwrap_or("Spotify")
        .to_string();
    let songs = entity
        .get("trackList")?
        .as_array()?
        .iter()
        .filter_map(|track| {
            let title = track.get("title")?.as_str()?.trim();
            let artist = track.get("subtitle")?.as_str()?.trim();
            (!title.is_empty()).then(|| Song {
                artist: artist.replace('\u{a0}', " "),
                title: title.to_string(),
            })
        })
        .collect();
    Some(SongList { name, songs })
}

pub fn csv(text: &str) -> Result<Vec<Song>, ApiError> {
    let mut rows = text.lines().filter(|line| !line.trim().is_empty());
    let header = split_row(
        rows.next()
            .ok_or_else(|| ApiError::invalid_input("the CSV file is empty"))?,
    );
    let column = |names: &[&str]| {
        header.iter().position(|cell| {
            let cell = cell.trim().trim_start_matches('\u{feff}').to_lowercase();
            names.iter().any(|name| cell == *name)
        })
    };
    let title = column(&["track name", "title", "track", "song", "name"])
        .ok_or_else(|| ApiError::invalid_input("the CSV has no track name column"))?;
    let artist = column(&["artist name(s)", "artist name", "artist", "artists"])
        .ok_or_else(|| ApiError::invalid_input("the CSV has no artist column"))?;
    Ok(rows
        .map(split_row)
        .filter_map(|cells| {
            let title = cells.get(title)?.trim().to_string();
            let artist = cells
                .get(artist)
                .map(|cell| cell.trim())
                .unwrap_or_default();
            (!title.is_empty()).then(|| Song {
                artist: artist.replace(';', ", "),
                title,
            })
        })
        .collect())
}

fn split_row(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => cells.push(std::mem::take(&mut cell)),
            (c, _) => cell.push(c),
        }
    }
    cells.push(cell);
    cells
}

fn fold(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn core_title(title: &str) -> String {
    let cut = title
        .find(['(', '['])
        .into_iter()
        .chain(title.find(" - "))
        .min()
        .unwrap_or(title.len());
    fold(&title[..cut])
}

pub fn pick(results: &[TrackInfo], song: &Song) -> Option<String> {
    let want = core_title(&song.title);
    if want.is_empty() {
        return None;
    }
    let first_artist = fold(song.artist.split(',').next().unwrap_or_default());
    let titled: Vec<&TrackInfo> = results
        .iter()
        .filter(|track| {
            let have = core_title(&track.title);
            !have.is_empty() && (have == want || have.contains(&want) || want.contains(&have))
        })
        .collect();
    titled
        .iter()
        .find(|track| !first_artist.is_empty() && fold(&track.artist).contains(&first_artist))
        .or_else(|| titled.first())
        .map(|track| track.key.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotify_links_name_their_kind_and_id() {
        assert_eq!(
            spotify_target("https://open.spotify.com/intl-tr/playlist/37i9dQZF1DXcBWIGoYBM5M?si=x"),
            Some(("playlist", "37i9dQZF1DXcBWIGoYBM5M".into()))
        );
        assert_eq!(
            spotify_target("https://open.spotify.com/album/4aawyAB9vmqN3uQ7FjRGTy"),
            Some(("album", "4aawyAB9vmqN3uQ7FjRGTy".into()))
        );
        assert_eq!(spotify_target("https://example.com/playlist/abc"), None);
        assert_eq!(spotify_target("https://open.spotify.com/track/abc"), None);
    }

    #[test]
    fn the_embed_page_gives_the_name_and_songs() {
        let page = r#"<html><script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"state":{"data":{"entity":{"name":"Mix","trackList":[{"title":"Housewife","subtitle":"Tate McRae"},{"title":"","subtitle":"x"}]}}}}}}</script>"#;
        let list = embed_songs(page).expect("songs");
        assert_eq!(list.name, "Mix");
        assert_eq!(
            list.songs,
            vec![Song {
                artist: "Tate McRae".into(),
                title: "Housewife".into()
            }]
        );
    }

    #[test]
    fn exportify_and_tunemymusic_csvs_both_read() {
        let exportify = "\u{feff}\"Track URI\",\"Track Name\",\"Artist Name(s)\"\n\"spotify:track:1\",\"Song, With Comma\",\"A;B\"\n";
        assert_eq!(
            csv(exportify).expect("rows"),
            vec![Song {
                artist: "A, B".into(),
                title: "Song, With Comma".into()
            }]
        );
        let tunemymusic = "Track name,Artist name,Album\nHello,Adele,25\n";
        assert_eq!(csv(tunemymusic).expect("rows")[0].artist, "Adele");
        assert!(csv("Foo,Bar\n1,2\n").is_err());
    }

    #[test]
    fn a_hit_is_matched_on_its_core_title_and_artist() {
        let track = |key: &str, title: &str, artist: &str| TrackInfo {
            key: key.into(),
            title: title.into(),
            artist: artist.into(),
            ..TrackInfo::default()
        };
        let results = [
            track("cover", "Housewife", "Someone Else"),
            track("real", "Housewife (Official Audio)", "Tate McRae"),
            track("other", "Different Song", "Tate McRae"),
        ];
        let song = Song {
            artist: "Tate McRae".into(),
            title: "Housewife - Remastered".into(),
        };
        assert_eq!(pick(&results, &song), Some("real".into()));
        let unknown = Song {
            artist: "Nobody".into(),
            title: "Nothing Like It".into(),
        };
        assert_eq!(pick(&results, &unknown), None);
    }
}

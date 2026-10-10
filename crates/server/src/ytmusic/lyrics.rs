//! A song's own lyrics from YouTube Music, read from its lyrics tab.
//!
//! The watch page names the tab (`MPLYt…`); browsing it as the Android app
//! gives timed lines, as the web client plain text. Both answers are walked
//! for the renderer that carries them rather than by fixed path, since the
//! wrapping around them moves between client versions.

use serde_json::{Value, json};

use super::clients::{ANDROID_MUSIC, WEB_REMIX};
use super::innertube;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedLine {
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YtLyrics {
    Timed(Vec<TimedLine>),
    Plain(String),
}

/// The lyrics YouTube Music shows for `video_id`, or `None` when it has none.
/// `cookies` only matter for a music video: its own lyrics tab is shut, and
/// YouTube names the song it is a video of only to a signed-in session.
pub async fn fetch(video_id: &str, cookies: Option<&str>) -> Result<Option<YtLyrics>, String> {
    let watch = watch_page(video_id, cookies).await?;
    let browse_id = match lyrics_browse_id(&watch) {
        Some(browse_id) => browse_id,
        None => {
            let Some(song) = counterpart_video_id(&watch, video_id) else {
                return Ok(None);
            };
            let Some(browse_id) = lyrics_browse_id(&watch_page(&song, cookies).await?) else {
                return Ok(None);
            };
            browse_id
        }
    };

    match innertube::post(
        ANDROID_MUSIC,
        "browse",
        json!({ "browseId": browse_id }),
        None,
    )
    .await
    {
        Ok(response) => {
            if let Some(lyrics) = android_lyrics(&response) {
                return Ok(Some(lyrics));
            }
        }
        Err(error) => tracing::debug!(%error, "timed lyrics unavailable, reading plain"),
    }
    let response = innertube::post(
        WEB_REMIX,
        "browse",
        json!({ "browseId": browse_id }),
        cookies,
    )
    .await?;
    Ok(plain_text(&response).map(YtLyrics::Plain))
}

async fn watch_page(video_id: &str, cookies: Option<&str>) -> Result<Value, String> {
    innertube::post(
        WEB_REMIX,
        "next",
        json!({ "videoId": video_id, "isAudioOnly": true }),
        cookies,
    )
    .await
}

/// Every object in `value`, depth first.
fn objects(value: &Value) -> Box<dyn Iterator<Item = &serde_json::Map<String, Value>> + '_> {
    match value {
        Value::Object(map) => Box::new(std::iter::once(map).chain(map.values().flat_map(objects))),
        Value::Array(items) => Box::new(items.iter().flat_map(objects)),
        _ => Box::new(std::iter::empty()),
    }
}

/// The lyrics tab's browse id. A tab YouTube marks unselectable has no lyrics
/// behind it, and its id leads to an empty page.
fn lyrics_browse_id(watch: &Value) -> Option<String> {
    objects(watch)
        .filter_map(|map| map.get("tabRenderer"))
        .filter(|tab| tab.get("unselectable").and_then(Value::as_bool) != Some(true))
        .filter_map(|tab| tab.pointer("/endpoint/browseEndpoint/browseId"))
        .filter_map(Value::as_str)
        .find(|id| id.starts_with("MPLYt"))
        .map(str::to_string)
}

fn milliseconds(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::String(text) => text.parse().ok(),
        other => other.as_u64(),
    }
}

/// The Android answer: timed lines, or the same model laid out statically
/// when the song only has unsynced words, which then read as plain text.
fn android_lyrics(response: &Value) -> Option<YtLyrics> {
    let data = objects(response).find(|map| map.contains_key("timedLyricsData"))?;
    let raw = data.get("timedLyricsData")?.as_array()?;
    let text = |line: &Value| {
        line.get("lyricLine")
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let timed: Option<Vec<TimedLine>> = raw
        .iter()
        .map(|line| {
            Some(TimedLine {
                start_ms: milliseconds(line.pointer("/cueRange/startTimeMilliseconds"))?,
                end_ms: milliseconds(line.pointer("/cueRange/endTimeMilliseconds")),
                text: text(line)?,
            })
        })
        .collect();
    let static_layout = data.get("staticLayout").and_then(Value::as_bool) == Some(true);
    match timed {
        Some(lines) if !static_layout && !lines.is_empty() => Some(YtLyrics::Timed(lines)),
        _ => {
            let plain = raw.iter().filter_map(text).collect::<Vec<_>>().join("\n");
            let plain = plain.trim();
            (!plain.is_empty()).then(|| YtLyrics::Plain(plain.to_string()))
        }
    }
}

/// The song a music video is a video of, as a signed-in watch page pairs them
/// in its queue.
fn counterpart_video_id(watch: &Value, video_id: &str) -> Option<String> {
    objects(watch)
        .filter_map(|map| map.get("playlistPanelVideoWrapperRenderer"))
        .filter(|wrapper| {
            wrapper
                .pointer("/primaryRenderer/playlistPanelVideoRenderer/videoId")
                .and_then(Value::as_str)
                == Some(video_id)
        })
        .filter_map(|wrapper| wrapper.get("counterpart")?.as_array())
        .flatten()
        .filter_map(|counterpart| {
            counterpart.pointer("/counterpartRenderer/playlistPanelVideoRenderer/videoId")
        })
        .filter_map(Value::as_str)
        .find(|id| *id != video_id)
        .map(str::to_string)
}

fn plain_text(response: &Value) -> Option<String> {
    let shelf = objects(response).find_map(|map| map.get("musicDescriptionShelfRenderer"))?;
    let text: String = shelf
        .pointer("/description/runs")?
        .as_array()?
        .iter()
        .filter_map(|run| run.get("text").and_then(Value::as_str))
        .collect();
    let text = text.replace("\r\n", "\n");
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_lyrics_tab_but_not_an_unselectable_one() {
        let tab = |id: &str, unselectable: bool| {
            json!({ "tabRenderer": {
                "unselectable": unselectable,
                "endpoint": { "browseEndpoint": { "browseId": id } }
            } })
        };
        let watch = json!({ "contents": { "tabs": [
            tab("MPTRt_up_next", false),
            tab("MPLYt_lyrics", false),
        ] } });
        assert_eq!(lyrics_browse_id(&watch).as_deref(), Some("MPLYt_lyrics"));

        let without = json!({ "tabs": [tab("MPLYt_none", true)] });
        assert_eq!(lyrics_browse_id(&without), None);
    }

    #[test]
    fn reads_timed_lines_wherever_they_sit() {
        let response = json!({ "contents": { "elementRenderer": { "newElement": {
            "type": { "componentType": { "model": { "timedLyricsModel": { "lyricsData": {
                "timedLyricsData": [
                    { "lyricLine": "First", "cueRange": {
                        "startTimeMilliseconds": "1200", "endTimeMilliseconds": "3400" } },
                    { "lyricLine": "Second", "cueRange": { "startTimeMilliseconds": 3400 } }
                ]
            } } } } }
        } } } });
        assert_eq!(
            android_lyrics(&response),
            Some(YtLyrics::Timed(vec![
                TimedLine {
                    start_ms: 1200,
                    end_ms: Some(3400),
                    text: "First".into()
                },
                TimedLine {
                    start_ms: 3400,
                    end_ms: None,
                    text: "Second".into()
                },
            ]))
        );
    }

    #[test]
    fn reads_plain_text_from_the_description_shelf() {
        let response = json!({ "contents": { "sectionListRenderer": { "contents": [
            { "musicDescriptionShelfRenderer": { "description": { "runs": [
                { "text": "Line one\n" }, { "text": "Line two" }
            ] } } }
        ] } } });
        assert_eq!(plain_text(&response).as_deref(), Some("Line one\nLine two"));
        assert_eq!(plain_text(&json!({ "contents": {} })), None);
    }

    fn fixture(name: &str) -> Value {
        let text = match name {
            "lyrics_android_synced" => include_str!("fixtures/lyrics_android_synced.json"),
            "lyrics_android_static" => include_str!("fixtures/lyrics_android_static.json"),
            "lyrics_web" => include_str!("fixtures/lyrics_web.json"),
            "next_music_video" => include_str!("fixtures/next_music_video.json"),
            _ => unreachable!("no fixture {name}"),
        };
        serde_json::from_str(text).expect("fixture is JSON")
    }

    #[test]
    fn reads_recorded_timed_lyrics() {
        let Some(YtLyrics::Timed(lines)) = android_lyrics(&fixture("lyrics_android_synced")) else {
            panic!("expected timed lyrics");
        };
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].text, "Hey Jude, don't make it bad");
        assert_eq!((lines[0].start_ms, lines[0].end_ms), (80, Some(6200)));
        assert!(
            lines
                .windows(2)
                .all(|pair| pair[1].start_ms > pair[0].start_ms)
        );
    }

    #[test]
    fn a_static_layout_reads_as_plain_text() {
        assert_eq!(
            android_lyrics(&fixture("lyrics_android_static")),
            Some(YtLyrics::Plain(
                "Let the music in tonight\nJust turn on the music\n\
                 Let the music of your life\nGive life back to music"
                    .into()
            ))
        );
    }

    #[test]
    fn reads_recorded_plain_lyrics() {
        assert_eq!(
            plain_text(&fixture("lyrics_web")).as_deref(),
            Some(
                "Let the music in tonight\nJust turn on the music\n\
                 Let the music of your life\nGive life back to music"
            )
        );
    }

    #[test]
    fn a_music_video_leads_to_its_song() {
        let watch = fixture("next_music_video");
        assert_eq!(lyrics_browse_id(&watch), None);
        assert_eq!(
            counterpart_video_id(&watch, "kJQP7kiw5Fk").as_deref(),
            Some("FXovf5dsRTw")
        );
        assert_eq!(counterpart_video_id(&watch, "FXovf5dsRTw"), None);
    }
}

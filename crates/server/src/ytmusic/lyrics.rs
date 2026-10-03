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
pub async fn fetch(video_id: &str, cookies: Option<&str>) -> Result<Option<YtLyrics>, String> {
    let watch = innertube::post(
        WEB_REMIX,
        "next",
        json!({ "videoId": video_id, "isAudioOnly": true }),
        cookies,
    )
    .await?;
    let Some(browse_id) = lyrics_browse_id(&watch) else {
        return Ok(None);
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
            if let Some(lines) = timed_lines(&response) {
                return Ok(Some(YtLyrics::Timed(lines)));
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

fn timed_lines(response: &Value) -> Option<Vec<TimedLine>> {
    let data = objects(response).find_map(|map| map.get("timedLyricsData"))?;
    let lines: Vec<TimedLine> = data
        .as_array()?
        .iter()
        .filter_map(|line| {
            Some(TimedLine {
                start_ms: milliseconds(line.pointer("/cueRange/startTimeMilliseconds"))?,
                end_ms: milliseconds(line.pointer("/cueRange/endTimeMilliseconds")),
                text: line.get("lyricLine")?.as_str()?.to_string(),
            })
        })
        .collect();
    (!lines.is_empty()).then_some(lines)
}

fn plain_text(response: &Value) -> Option<String> {
    let shelf = objects(response).find_map(|map| map.get("musicDescriptionShelfRenderer"))?;
    let text: String = shelf
        .pointer("/description/runs")?
        .as_array()?
        .iter()
        .filter_map(|run| run.get("text").and_then(Value::as_str))
        .collect();
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
            timed_lines(&response),
            Some(vec![
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
            ])
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
}

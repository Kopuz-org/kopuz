//! An example lyrics provider: asks the address the person gives it for the
//! words to a song, as LRC or plain text.

use kopuz_plugin_sdk::SourceError;
use kopuz_plugin_sdk::http::Client;
use kopuz_plugin_sdk::integration::{FoundLyrics, Integration, Settings, Song};

pub struct LyricsServer {
    http: Client,
    url: String,
}

impl Integration for LyricsServer {
    fn open(settings: Settings) -> Self {
        LyricsServer {
            http: Client::new(),
            url: settings.get("url").unwrap_or_default().to_string(),
        }
    }

    fn validate(&self) -> Result<(), SourceError> {
        if self.url.is_empty() {
            return Err(SourceError::InvalidInput(
                "the lyrics server needs an address".into(),
            ));
        }
        // Any answer but a server error means something is listening there.
        let response = self.http.get(&self.url).send()?;
        match response.status() {
            500.. => Err(SourceError::from_status(
                response.status(),
                "the lyrics server",
            )),
            _ => Ok(()),
        }
    }

    fn lyrics(&self, track: Song) -> Result<Option<FoundLyrics>, String> {
        let seconds = track.duration_ms.map(|ms| (ms / 1_000).to_string());
        let mut query = vec![
            ("artist", track.artist.as_str()),
            ("title", track.title.as_str()),
            ("album", track.album.as_str()),
        ];
        // Left out rather than sent empty when the length is not known.
        if let Some(seconds) = &seconds {
            query.push(("duration", seconds.as_str()));
        }
        let response = self
            .http
            .get(&self.url)
            .query(&query)
            .send()
            .map_err(|error| error.to_string())?;
        match response.status() {
            404 => return Ok(None),
            status if !(200..300).contains(&status) => {
                return Err(format!("the lyrics server answered {status}"));
            }
            _ => {}
        }
        Ok(found(&response.text()))
    }
}

/// LRC when a line starts with a time tag, else plain words; nothing for an empty answer.
fn found(text: &str) -> Option<FoundLyrics> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let timed = text.lines().any(|line| {
        line.trim_start()
            .strip_prefix('[')
            .and_then(|rest| rest.chars().next())
            .is_some_and(|first| first.is_ascii_digit())
    });
    Some(match timed {
        true => FoundLyrics::Lrc(text.to_string()),
        false => FoundLyrics::Plain(text.to_string()),
    })
}

kopuz_plugin_sdk::export_integration!(LyricsServer);

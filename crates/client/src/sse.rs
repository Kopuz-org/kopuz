//! Incremental SSE parsing and the reconnecting event loop.

use std::time::Duration;

use api::ApiEvent;
use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Default)]
pub(crate) struct SseParser {
    buffer: String,
    name: Option<String>,
    data: Vec<String>,
    id: Option<u64>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct SseFrame {
    pub name: String,
    pub data: String,
    pub id: Option<u64>,
}

impl SseParser {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseFrame> {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        let mut frames = Vec::new();
        while let Some(position) = self.buffer.find('\n') {
            let line: String = self.buffer.drain(..=position).collect();
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if let Some(name) = self.name.take() {
                    frames.push(SseFrame {
                        name,
                        data: self.data.join("\n"),
                        id: self.id,
                    });
                }
                self.data.clear();
            } else if let Some(value) = field(line, "event") {
                self.name = Some(value.to_string());
            } else if let Some(value) = field(line, "data") {
                self.data.push(value.to_string());
            } else if let Some(value) = field(line, "id") {
                self.id = value.parse().ok();
            }
        }
        frames
    }
}

fn field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(name)?.strip_prefix(':')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

/// Frame to typed event. Unknown names return `None` and are skipped, so new
/// daemon event types never break older clients.
pub(crate) fn frame_to_event(frame: &SseFrame) -> Option<ApiEvent> {
    if frame.name == "resync" {
        return Some(ApiEvent::Resync);
    }
    let mut value = serde_json::Map::new();
    value.insert("event".into(), frame.name.clone().into());
    if !frame.data.is_empty()
        && let Ok(data) = serde_json::from_str::<serde_json::Value>(&frame.data)
        && !data.is_null()
    {
        value.insert("data".into(), data);
    }
    serde_json::from_value(serde_json::Value::Object(value)).ok()
}

pub(crate) async fn run_event_loop(
    client: reqwest::Client,
    url: String,
    token: String,
    tx: UnboundedSender<ApiEvent>,
) {
    let mut last_id: Option<u64> = None;
    loop {
        let mut request = client
            .get(&url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"));
        if let Some(id) = last_id {
            request = request.header("Last-Event-ID", id.to_string());
        }
        match request.send().await {
            Ok(response) if response.status().is_success() => {
                let mut stream = response.bytes_stream();
                let mut parser = SseParser::default();
                while let Some(chunk) = stream.next().await {
                    let Ok(chunk) = chunk else { break };
                    for frame in parser.feed(&chunk) {
                        if let Some(id) = frame.id {
                            last_id = Some(id);
                        }
                        if let Some(event) = frame_to_event(&frame)
                            && tx.send(event).is_err()
                        {
                            return;
                        }
                    }
                }
            }
            Ok(response) => {
                tracing::warn!(status = %response.status(), "event stream rejected");
            }
            Err(error) => {
                tracing::debug!(%error, "event stream connect failed");
            }
        }
        if tx.is_closed() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_handles_frames_split_across_chunks() {
        let mut parser = SseParser::default();
        assert!(parser.feed(b"id: 7\nevent: queue.ch").is_empty());
        let frames = parser.feed(b"anged\ndata: {\"rev\":3}\n\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].name, "queue.changed");
        assert_eq!(frames[0].data, "{\"rev\":3}");
        assert_eq!(frames[0].id, Some(7));
    }

    #[test]
    fn parser_ignores_comment_keepalives() {
        let mut parser = SseParser::default();
        assert!(parser.feed(b": keep-alive\n\n").is_empty());
    }

    #[test]
    fn frames_become_typed_events_and_unknown_names_are_skipped() {
        let frame = SseFrame {
            name: "library.invalidated".into(),
            data: "{\"table\":\"tracks\",\"generation\":9}".into(),
            id: Some(1),
        };
        assert_eq!(
            frame_to_event(&frame),
            Some(ApiEvent::LibraryInvalidated {
                table: api::Table::Tracks,
                generation: 9,
            })
        );

        let resync = SseFrame {
            name: "resync".into(),
            data: "{}".into(),
            id: None,
        };
        assert_eq!(frame_to_event(&resync), Some(ApiEvent::Resync));

        let unknown = SseFrame {
            name: "job.telepathy".into(),
            data: "{}".into(),
            id: None,
        };
        assert_eq!(frame_to_event(&unknown), None);
    }
}

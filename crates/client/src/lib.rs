//! `HttpApi`: the wire twin of the daemon's in-process `LocalApi`.
//!
//! Implements [`api::KopuzApi`] over the daemon's HTTP/JSON + SSE surface, so
//! a Rust frontend can swap between embedding the daemon and attaching to a
//! remote one without touching its data layer. The contract tests in the
//! daemon crate run the same assertions through both implementations.

mod sse;

use api::{
    ApiError, CommandAck, ErrorBody, KopuzApi, Page, PlayerCommand, PlayerState, QueueEdit,
    QueueWindow, SetQueueRequest, TrackFilter, TrackPage,
};
use serde::de::DeserializeOwned;

pub struct HttpApi {
    base: String,
    token: String,
    client: reqwest::Client,
}

impl HttpApi {
    pub fn base_url(&self) -> &str {
        &self.base
    }

    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Self {
        let mut base = base.into();
        while base.ends_with('/') {
            base.pop();
        }
        Self {
            base,
            token: token.into(),
            client: reqwest::Client::new(),
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("{}{}", self.base, path))
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.token),
            )
    }

    async fn send<T: DeserializeOwned>(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<T, ApiError> {
        let response = builder
            .send()
            .await
            .map_err(|error| ApiError::internal(format!("daemon unreachable: {error}")))?;
        let status = response.status();
        if status.is_success() {
            response
                .json()
                .await
                .map_err(|error| ApiError::internal(format!("malformed daemon response: {error}")))
        } else {
            #[derive(serde::Deserialize)]
            struct Failure {
                error: ErrorBody,
            }
            match response.json::<Failure>().await {
                Ok(failure) => Err(failure.error.into()),
                Err(_) => Err(ApiError::internal(format!("daemon returned {status}"))),
            }
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        self.send(self.request(reqwest::Method::GET, path)).await
    }

    async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T, ApiError> {
        let mut builder = self.request(reqwest::Method::POST, path);
        if let Some(body) = body {
            builder = builder.json(&body);
        }
        self.send(builder).await
    }
}

#[async_trait::async_trait]
impl KopuzApi for HttpApi {
    async fn player_state(&self) -> Result<PlayerState, ApiError> {
        self.get("/v1/player").await
    }

    async fn player_command(&self, command: PlayerCommand) -> Result<CommandAck, ApiError> {
        let (path, body) = match command {
            PlayerCommand::Play => ("/v1/player/play", None),
            PlayerCommand::Pause => ("/v1/player/pause", None),
            PlayerCommand::Toggle => ("/v1/player/toggle", None),
            PlayerCommand::Next => ("/v1/player/next", None),
            PlayerCommand::Previous => ("/v1/player/previous", None),
            PlayerCommand::Stop => ("/v1/player/stop", None),
            PlayerCommand::Seek { position_ms } => (
                "/v1/player/seek",
                Some(serde_json::json!({ "position_ms": position_ms })),
            ),
            PlayerCommand::SetVolume { volume } => (
                "/v1/player/volume",
                Some(serde_json::json!({ "volume": volume })),
            ),
            PlayerCommand::SetMode { shuffle, loop_mode } => {
                let mut body = serde_json::Map::new();
                if let Some(shuffle) = shuffle {
                    body.insert("shuffle".into(), shuffle.into());
                }
                if let Some(mode) = loop_mode {
                    body.insert(
                        "loop".into(),
                        serde_json::to_value(mode)
                            .map_err(|error| ApiError::internal(error.to_string()))?,
                    );
                }
                ("/v1/player/mode", Some(serde_json::Value::Object(body)))
            }
        };
        self.post(path, body).await
    }

    async fn queue_window(&self, page: Page) -> Result<QueueWindow, ApiError> {
        self.get(&format!(
            "/v1/queue?offset={}&limit={}",
            page.offset, page.limit
        ))
        .await
    }

    async fn set_queue(&self, request: SetQueueRequest) -> Result<CommandAck, ApiError> {
        let body = serde_json::to_value(&request)
            .map_err(|error| ApiError::internal(error.to_string()))?;
        self.post("/v1/queue", Some(body)).await
    }

    async fn queue_edit(&self, edit: QueueEdit) -> Result<CommandAck, ApiError> {
        match edit {
            QueueEdit::Jump { index } => {
                self.post(
                    "/v1/queue/jump",
                    Some(serde_json::json!({ "index": index })),
                )
                .await
            }
            QueueEdit::Move { from, to } => {
                self.post(
                    "/v1/queue/move",
                    Some(serde_json::json!({ "from": from, "to": to })),
                )
                .await
            }
            QueueEdit::Remove { index } => {
                self.send(
                    self.request(reqwest::Method::DELETE, &format!("/v1/queue/items/{index}")),
                )
                .await
            }
        }
    }

    async fn tracks(&self, filter: TrackFilter, page: Page) -> Result<TrackPage, ApiError> {
        let mut query = vec![
            ("offset", page.offset.to_string()),
            ("limit", page.limit.to_string()),
        ];
        let mut push = |name: &'static str, value: Option<String>| {
            if let Some(value) = value {
                query.push((name, value));
            }
        };
        push("search", filter.search);
        push("artist", filter.artist);
        push("album", filter.album);
        push("genre", filter.genre);
        push("favorite", filter.favorite.map(|f| f.to_string()));
        push("sort", filter.sort);
        let pairs: Vec<String> = query
            .into_iter()
            .map(|(name, value)| format!("{name}={}", urlencode(&value)))
            .collect();
        self.get(&format!("/v1/library/tracks?{}", pairs.join("&")))
            .await
    }

    /// Connects to `/v1/events`, reconnecting with `Last-Event-ID` after
    /// drops. Unknown event types are skipped, matching the protocol's
    /// forward-compatibility rule; a gap past the daemon's replay ring
    /// surfaces as `ApiEvent::Resync`.
    fn events(&self) -> api::EventStream {
        use futures_util::StreamExt;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let client = self.client.clone();
        let url = format!("{}/v1/events", self.base);
        let token = self.token.clone();
        tokio::spawn(sse::run_event_loop(client, url, token, tx));
        futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|event| (event, rx))
        })
        .boxed()
    }
}

fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::urlencode;

    #[test]
    fn urlencode_escapes_reserved_bytes() {
        assert_eq!(urlencode("plain-text_1.0~x"), "plain-text_1.0~x");
        assert_eq!(urlencode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(urlencode("ü"), "%C3%BC");
    }
}

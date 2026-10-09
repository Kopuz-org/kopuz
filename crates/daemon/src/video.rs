//! The picture of a music video, served to a frontend a range at a time.
//!
//! The source's stream URL is signed for this session and never leaves the
//! daemon: a frontend asks for bytes by track key, the way it asks
//! [`crate::ArtworkService`] for a cover, and its video element plays them
//! muted while the engine plays the sound.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::{ApiError, VideoChunk, VideoRequest};
use server::source::VideoStream;

use crate::session::SessionHandle;

/// What one request answers at most. A webview asks again from where it got
/// to, so this bounds memory per request rather than what can be watched.
const MAX_CHUNK: u64 = 2 * 1024 * 1024;
/// googlevideo URLs live for hours; resolving again well before that costs
/// one player call per half hour of watching.
const STREAM_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_STREAMS: usize = 8;

pub struct VideoService {
    db: db::Db,
    session: SessionHandle,
    http: reqwest::Client,
    streams: Mutex<HashMap<String, (VideoStream, Instant)>>,
}

/// What a range request ended in, short of bytes.
enum Failure {
    Api(ApiError),
    /// The remote's own refusal; 403 and 410 mean the URL went stale.
    Status(reqwest::StatusCode),
}

impl Failure {
    fn stale(&self) -> bool {
        matches!(self, Self::Status(status) if matches!(status.as_u16(), 403 | 410))
    }

    fn into_api(self) -> ApiError {
        match self {
            Self::Api(error) => error,
            Self::Status(status) => ApiError::new(
                api::ErrorCode::SourceUnreachable,
                format!("the video stream answered {status}"),
            ),
        }
    }
}

impl VideoService {
    pub fn new(db: db::Db, session: SessionHandle) -> Arc<Self> {
        Arc::new(Self {
            db,
            session,
            http: reqwest::Client::new(),
            streams: Mutex::new(HashMap::new()),
        })
    }

    pub async fn chunk(&self, request: VideoRequest) -> Result<VideoChunk, ApiError> {
        let track = self
            .session
            .queued_track(&request.key)
            .await
            .ok_or_else(|| ApiError::not_found("that track is not queued"))?;
        if track.counterpart.as_deref().is_none_or(|other| other.video) {
            return Err(ApiError::invalid_input("that track is not a music video"));
        }
        let item_id = track.id.key().into_owned();
        let length = request.length.unwrap_or(MAX_CHUNK).clamp(1, MAX_CHUNK);
        tracing::debug!(start = request.start, length, "video range");
        let stream = self.stream(&item_id, false).await?;
        match self.fetch(&stream, request.start, length).await {
            Err(failure) if failure.stale() => {
                let stream = self.stream(&item_id, true).await?;
                self.fetch(&stream, request.start, length)
                    .await
                    .map_err(Failure::into_api)
            }
            other => other.map_err(Failure::into_api),
        }
    }

    async fn stream(&self, item_id: &str, refresh: bool) -> Result<VideoStream, ApiError> {
        if !refresh
            && let Ok(streams) = self.streams.lock()
            && let Some((stream, at)) = streams.get(item_id)
            && at.elapsed() < STREAM_TTL
        {
            return Ok(stream.clone());
        }
        let config = self.session.config_watch().borrow().clone();
        let source = server::source::active(self.db.clone(), &config);
        if !source.capabilities().music_videos {
            return Err(ApiError::unsupported("this source has no music videos"));
        }
        let stream = source
            .video_stream(item_id)
            .await
            .map_err(crate::catalog::source_error)?;
        if let Ok(mut streams) = self.streams.lock() {
            streams.retain(|_, (_, at)| at.elapsed() < STREAM_TTL);
            if streams.len() >= MAX_STREAMS {
                streams.clear();
            }
            streams.insert(item_id.to_string(), (stream.clone(), Instant::now()));
        }
        Ok(stream)
    }

    async fn fetch(
        &self,
        stream: &VideoStream,
        start: u64,
        length: u64,
    ) -> Result<VideoChunk, Failure> {
        let chunk = |total, bytes| VideoChunk {
            content_type: stream.content_type.clone(),
            start,
            total,
            bytes,
        };
        if stream.content_length.is_some_and(|total| start >= total) {
            return Ok(chunk(stream.content_length, Vec::new()));
        }
        let end = start + length - 1;
        let end = stream
            .content_length
            .map_or(end, |total| end.min(total.saturating_sub(1)));
        let mut request = self
            .http
            .get(&stream.url)
            .header(reqwest::header::RANGE, format!("bytes={start}-{end}"));
        if let Some(agent) = &stream.user_agent {
            request = request.header(reqwest::header::USER_AGENT, agent);
        }
        let unreachable = |_| {
            Failure::Api(ApiError::new(
                api::ErrorCode::SourceUnreachable,
                "the video stream could not be reached",
            ))
        };
        let response = request.send().await.map_err(unreachable)?;
        let status = response.status();
        if !status.is_success() {
            return Err(Failure::Status(status));
        }
        let total = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit('/').next())
            .and_then(|total| total.parse().ok())
            .or(stream.content_length);
        let bytes = response.bytes().await.map_err(unreachable)?;
        Ok(chunk(total, bytes.to_vec()))
    }
}

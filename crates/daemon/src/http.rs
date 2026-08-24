//! Minimal axum shell over a running session: the Phase 2 skeleton.
//!
//! Serves the playback/queue routes, an SSE event stream with `Last-Event-ID`
//! replay (a client that reconnects past the ring gets one `resync` event and
//! refetches its snapshots), and bearer-token auth. Interim shape by design:
//! no Origin allowlist yet, so bind loopback only until that lands.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::{
    ApiError, ApiEvent, ErrorCode, KopuzApi, Page, PlayerCommand, QueueEdit, SetQueueRequest,
};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures_util::{Stream, StreamExt};
use tokio::sync::broadcast;

use crate::session::SessionHandle;

pub struct HttpState {
    pub api: Arc<dyn KopuzApi>,
    /// Entity-addressed artwork; `None` disables the endpoint.
    pub artwork: Option<Arc<crate::artwork::ArtworkService>>,
    /// Event source with sequence numbers and the replay ring; the trait's
    /// `events()` strips ids, and SSE resume needs them.
    pub session: SessionHandle,
    pub token: String,
    /// Browser origins allowed to call the API. Non-browser clients send no
    /// Origin header and are unaffected; any unlisted Origin is refused even
    /// with a valid token, so a malicious web page cannot ride a leaked one.
    pub allowed_origins: Vec<String>,
    pub started: Instant,
}

pub fn router(state: Arc<HttpState>) -> Router {
    Router::new()
        .route("/v1/status", get(status))
        .route("/v1/player", get(player_state))
        .route("/v1/player/play", post(play))
        .route("/v1/player/pause", post(pause))
        .route("/v1/player/toggle", post(toggle))
        .route("/v1/player/next", post(next_track))
        .route("/v1/player/previous", post(previous_track))
        .route("/v1/player/stop", post(stop))
        .route("/v1/player/seek", post(seek))
        .route("/v1/player/volume", post(volume))
        .route("/v1/player/mode", post(mode))
        .route("/v1/config", get(get_config).patch(patch_config))
        .route("/v1/favorites", get(get_favorites).put(put_favorite))
        .route("/v1/favorites/sync", post(favorites_sync))
        .route("/v1/library/scan", post(library_scan))
        .route("/v1/library/sync", post(library_sync))
        .route(
            "/v1/downloads",
            get(list_downloads)
                .post(start_downloads)
                .delete(remove_download),
        )
        .route("/v1/jobs", get(list_jobs))
        .route("/v1/jobs/{id}/cancel", post(cancel_job))
        .route("/v1/library/tracks", get(library_tracks))
        .route("/v1/library/folders", get(library_folders))
        .route("/v1/library/stats", get(library_stats))
        .route("/v1/lyrics", get(lyrics))
        .route("/v1/queue", get(queue_window).post(set_queue))
        .route("/v1/queue/jump", post(queue_jump))
        .route("/v1/queue/move", post(queue_move))
        .route("/v1/queue/items/{index}", delete(queue_remove))
        .route("/v1/artwork", get(artwork))
        .route("/v1/events", get(events))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state)
}

pub async fn serve(
    listener: tokio::net::TcpListener,
    state: Arc<HttpState>,
) -> std::io::Result<()> {
    axum::serve(listener, router(state)).await
}

struct ApiFailure(ApiError);

impl From<ApiError> for ApiFailure {
    fn from(error: ApiError) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiFailure {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.code.http_status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(serde_json::json!({ "error": self.0.body() }))).into_response()
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn query_param<'a>(query: Option<&'a str>, name: &str) -> Option<&'a str> {
    query?
        .split('&')
        .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
}

/// Bearer auth on every route. `EventSource` cannot set headers, so the token
/// is also accepted as a `?token=` query parameter.
async fn auth(State(state): State<Arc<HttpState>>, request: Request, next: Next) -> Response {
    if let Some(origin) = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        && !state
            .allowed_origins
            .iter()
            .any(|allowed| allowed == origin)
    {
        return ApiFailure(ApiError::new(ErrorCode::Unauthorized, "origin not allowed"))
            .into_response();
    }
    let header_token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let provided = header_token.or_else(|| query_param(request.uri().query(), "token"));
    if provided.is_some_and(|token| constant_time_eq(token.as_bytes(), state.token.as_bytes())) {
        next.run(request).await
    } else {
        ApiFailure(ApiError::new(
            ErrorCode::Unauthorized,
            "missing or invalid bearer token",
        ))
        .into_response()
    }
}

async fn status(State(state): State<Arc<HttpState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "api_version": api::API_VERSION,
        "uptime_secs": state.started.elapsed().as_secs(),
    }))
}

async fn player_state(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    Ok(Json(state.api.player_state().await?).into_response())
}

async fn command(state: Arc<HttpState>, command: PlayerCommand) -> Result<Response, ApiFailure> {
    Ok(Json(state.api.player_command(command).await?).into_response())
}

async fn play(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    command(state, PlayerCommand::Play).await
}

async fn pause(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    command(state, PlayerCommand::Pause).await
}

async fn toggle(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    command(state, PlayerCommand::Toggle).await
}

async fn next_track(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    command(state, PlayerCommand::Next).await
}

async fn previous_track(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    command(state, PlayerCommand::Previous).await
}

async fn stop(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    command(state, PlayerCommand::Stop).await
}

#[derive(serde::Deserialize)]
struct SeekBody {
    position_ms: u64,
}

async fn seek(
    State(state): State<Arc<HttpState>>,
    Json(body): Json<SeekBody>,
) -> Result<Response, ApiFailure> {
    command(
        state,
        PlayerCommand::Seek {
            position_ms: body.position_ms,
        },
    )
    .await
}

#[derive(serde::Deserialize)]
struct VolumeBody {
    volume: f32,
}

async fn volume(
    State(state): State<Arc<HttpState>>,
    Json(body): Json<VolumeBody>,
) -> Result<Response, ApiFailure> {
    command(
        state,
        PlayerCommand::SetVolume {
            volume: body.volume,
        },
    )
    .await
}

#[derive(serde::Deserialize)]
struct ModeBody {
    shuffle: Option<bool>,
    #[serde(rename = "loop")]
    loop_mode: Option<api::LoopMode>,
}

async fn mode(
    State(state): State<Arc<HttpState>>,
    Json(body): Json<ModeBody>,
) -> Result<Response, ApiFailure> {
    command(
        state,
        PlayerCommand::SetMode {
            shuffle: body.shuffle,
            loop_mode: body.loop_mode,
        },
    )
    .await
}

#[derive(serde::Deserialize)]
struct PageQuery {
    offset: Option<u32>,
    limit: Option<u32>,
}

async fn queue_window(
    State(state): State<Arc<HttpState>>,
    Query(page): Query<PageQuery>,
) -> Result<Response, ApiFailure> {
    let page = Page {
        offset: page.offset.unwrap_or(0),
        limit: page.limit.unwrap_or(api::DEFAULT_PAGE_LIMIT),
    };
    Ok(Json(state.api.queue_window(page).await?).into_response())
}

#[derive(serde::Deserialize)]
struct FolderQuery {
    prefix: String,
    offset: Option<u32>,
    limit: Option<u32>,
}

async fn library_folders(
    State(state): State<Arc<HttpState>>,
    Query(query): Query<FolderQuery>,
) -> Result<Response, ApiFailure> {
    let page = Page {
        offset: query.offset.unwrap_or(0),
        limit: query.limit.unwrap_or(api::DEFAULT_PAGE_LIMIT),
    };
    Ok(Json(state.api.folder_tracks(query.prefix, page).await?).into_response())
}

async fn library_stats(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    Ok(Json(state.api.stats().await?).into_response())
}

#[derive(serde::Deserialize)]
struct LyricsQuery {
    track: String,
}

async fn lyrics(
    State(state): State<Arc<HttpState>>,
    Query(query): Query<LyricsQuery>,
) -> Result<Response, ApiFailure> {
    Ok(Json(state.api.lyrics(query.track).await?).into_response())
}

#[derive(serde::Deserialize)]
struct TracksQuery {
    search: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    genre: Option<String>,
    favorite: Option<bool>,
    sort: Option<String>,
    offset: Option<u32>,
    limit: Option<u32>,
}

async fn get_config(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    Ok(Json(state.api.config().await?).into_response())
}

async fn patch_config(
    State(state): State<Arc<HttpState>>,
    Json(patch): Json<serde_json::Value>,
) -> Result<Response, ApiFailure> {
    Ok(Json(state.api.patch_config(patch).await?).into_response())
}

async fn get_favorites(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    Ok(Json(state.api.favorites().await?).into_response())
}

#[derive(serde::Deserialize)]
struct FavoriteBody {
    key: String,
    favorite: bool,
}

async fn put_favorite(
    State(state): State<Arc<HttpState>>,
    Json(body): Json<FavoriteBody>,
) -> Result<Response, ApiFailure> {
    state.api.set_favorite(body.key, body.favorite).await?;
    Ok(Json(serde_json::json!({})).into_response())
}

async fn start_job(state: Arc<HttpState>, kind: api::JobKind) -> Result<Response, ApiFailure> {
    Ok(Json(state.api.start_job(kind).await?).into_response())
}

async fn favorites_sync(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    start_job(state, api::JobKind::FavoritesSync).await
}

async fn library_scan(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    start_job(state, api::JobKind::Scan).await
}

async fn library_sync(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    start_job(state, api::JobKind::LibrarySync).await
}

#[derive(serde::Deserialize)]
struct DownloadBody {
    keys: Vec<String>,
}

async fn start_downloads(
    State(state): State<Arc<HttpState>>,
    Json(body): Json<DownloadBody>,
) -> Result<Response, ApiFailure> {
    Ok(Json(state.api.download(body.keys).await?).into_response())
}

async fn list_downloads(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    Ok(Json(state.api.downloads().await?).into_response())
}

#[derive(serde::Deserialize)]
struct RemoveDownloadBody {
    key: String,
}

async fn remove_download(
    State(state): State<Arc<HttpState>>,
    Json(body): Json<RemoveDownloadBody>,
) -> Result<Response, ApiFailure> {
    state.api.remove_download(body.key).await?;
    Ok(Json(serde_json::json!({})).into_response())
}

async fn list_jobs(State(state): State<Arc<HttpState>>) -> Result<Response, ApiFailure> {
    Ok(Json(state.api.jobs().await?).into_response())
}

async fn cancel_job(
    State(state): State<Arc<HttpState>>,
    Path(id): Path<String>,
) -> Result<Response, ApiFailure> {
    state.api.cancel_job(id).await?;
    Ok(Json(serde_json::json!({})).into_response())
}

async fn library_tracks(
    State(state): State<Arc<HttpState>>,
    Query(query): Query<TracksQuery>,
) -> Result<Response, ApiFailure> {
    let filter = api::TrackFilter {
        search: query.search,
        artist: query.artist,
        album: query.album,
        genre: query.genre,
        favorite: query.favorite,
        sort: query.sort,
    };
    let page = Page {
        offset: query.offset.unwrap_or(0),
        limit: query.limit.unwrap_or(api::DEFAULT_PAGE_LIMIT),
    };
    Ok(Json(state.api.tracks(filter, page).await?).into_response())
}

async fn set_queue(
    State(state): State<Arc<HttpState>>,
    Json(request): Json<SetQueueRequest>,
) -> Result<Response, ApiFailure> {
    Ok(Json(state.api.set_queue(request).await?).into_response())
}

#[derive(serde::Deserialize)]
struct JumpBody {
    index: u32,
}

async fn queue_jump(
    State(state): State<Arc<HttpState>>,
    Json(body): Json<JumpBody>,
) -> Result<Response, ApiFailure> {
    Ok(Json(
        state
            .api
            .queue_edit(QueueEdit::Jump { index: body.index })
            .await?,
    )
    .into_response())
}

#[derive(serde::Deserialize)]
struct MoveBody {
    from: u32,
    to: u32,
}

async fn queue_move(
    State(state): State<Arc<HttpState>>,
    Json(body): Json<MoveBody>,
) -> Result<Response, ApiFailure> {
    Ok(Json(
        state
            .api
            .queue_edit(QueueEdit::Move {
                from: body.from,
                to: body.to,
            })
            .await?,
    )
    .into_response())
}

async fn queue_remove(
    State(state): State<Arc<HttpState>>,
    Path(index): Path<u32>,
) -> Result<Response, ApiFailure> {
    Ok(Json(state.api.queue_edit(QueueEdit::Remove { index }).await?).into_response())
}

fn sse_event(sequence: u64, event: &ApiEvent) -> SseEvent {
    let value = serde_json::to_value(event).unwrap_or(serde_json::Value::Null);
    let name = value
        .get("event")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("message")
        .to_string();
    let data = value
        .get("data")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    SseEvent::default()
        .id(sequence.to_string())
        .event(name)
        .data(data.to_string())
}

#[derive(serde::Deserialize)]
struct ArtworkQuery {
    track: Option<String>,
    album: Option<String>,
    artist: Option<String>,
    #[serde(default)]
    hq: bool,
}

async fn artwork(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Query(query): Query<ArtworkQuery>,
) -> Result<Response, ApiFailure> {
    use crate::artwork::ArtworkEntity;
    let Some(service) = &state.artwork else {
        return Err(ApiFailure(ApiError::unsupported(
            "this daemon runs without artwork",
        )));
    };
    let entity = if let Some(track) = query.track.as_deref() {
        ArtworkEntity::Track(track)
    } else if let Some(album) = query.album.as_deref() {
        ArtworkEntity::Album(album)
    } else if let Some(artist) = query.artist.as_deref() {
        ArtworkEntity::Artist(artist)
    } else {
        return Err(ApiFailure(ApiError::invalid_input(
            "pass one of track, album, or artist",
        )));
    };
    let payload = service.fetch(entity, query.hq).await?;
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        == Some(payload.etag.as_str())
    {
        return Ok(StatusCode::NOT_MODIFIED.into_response());
    }
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, payload.content_type.to_string()),
            (header::ETAG, payload.etag),
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000".to_string(),
            ),
        ],
        payload.bytes,
    )
        .into_response())
}

fn resync_event() -> SseEvent {
    SseEvent::default().event("resync").data("{}")
}

fn live_stream(
    rx: broadcast::Receiver<(u64, ApiEvent)>,
    floor: u64,
) -> impl Stream<Item = Result<SseEvent, Infallible>> {
    futures_util::stream::unfold((rx, floor), |(mut rx, floor)| async move {
        loop {
            match rx.recv().await {
                Ok((sequence, event)) => {
                    if sequence <= floor {
                        continue;
                    }
                    return Some((Ok(sse_event(sequence, &event)), (rx, floor)));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    return Some((Ok(resync_event()), (rx, floor)));
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
}

/// SSE with `Last-Event-ID` resume: subscribe first, then replay the ring for
/// the gap (deduplicating by sequence), or hand the client one `resync` event
/// when the ring no longer reaches back that far.
async fn events(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let last = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let live = state.session.subscribe();
    let (needs_resync, replayed) = match last {
        Some(last) => state.session.replay_since(last),
        None => (false, Vec::new()),
    };
    let floor = replayed
        .last()
        .map(|(sequence, _)| *sequence)
        .or(if needs_resync { None } else { last })
        .unwrap_or(0);
    let mut prefix: Vec<Result<SseEvent, Infallible>> = Vec::with_capacity(replayed.len() + 1);
    if needs_resync {
        prefix.push(Ok(resync_event()));
    }
    prefix.extend(
        replayed
            .iter()
            .map(|(sequence, event)| Ok(sse_event(*sequence, event))),
    );
    let stream = futures_util::stream::iter(prefix).chain(live_stream(live, floor));
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_param_finds_token_anywhere_in_the_query() {
        assert_eq!(query_param(Some("token=abc"), "token"), Some("abc"));
        assert_eq!(query_param(Some("a=1&token=abc&b=2"), "token"), Some("abc"));
        assert_eq!(query_param(Some("tokenish=abc"), "token"), None);
        assert_eq!(query_param(None, "token"), None);
    }

    #[test]
    fn sse_event_uses_dotted_names_and_flat_payloads() {
        let event = ApiEvent::LibraryInvalidated {
            table: api::Table::Tracks,
            generation: 3,
        };
        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["event"], "library.invalidated");
    }
}

//! Linux's music video transport. WebKitGTK hands a media URL to GStreamer,
//! whose WebKit source element takes only http(s) and blob, so `artwork://`
//! fails to load with a FormatError. Loopback HTTP serves the same ranges.

use axum::{
    Router,
    body::{Body, Bytes},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::{StreamExt, stream};
use http::{StatusCode, header};

use crate::artwork_protocol::{byte_range, video_blocks, video_key};

pub async fn start() -> std::io::Result<String> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    // Only this process knows the route; the stream is the user's signed session.
    let path = format!("/{}/video", uuid::Uuid::new_v4());
    let endpoint = format!("http://{}{path}", listener.local_addr()?);
    let router = Router::new().route(&path, get(video));
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            tracing::error!(%error, "video server stopped");
        }
    });
    Ok(endpoint)
}

async fn video(uri: http::Uri, headers: http::HeaderMap) -> Response {
    let query = uri.query().unwrap_or_default();
    let range = headers
        .get(header::RANGE)
        .and_then(|range| range.to_str().ok());
    let one_range = || async {
        crate::artwork_protocol::video(query, range)
            .await
            .map(Body::from)
            .into_response()
    };
    // GStreamer asks for open ranges and takes the end of a body for the end of
    // the video, so one block answered alone plays a few seconds and stops.
    let (start, end) = range.and_then(byte_range).unwrap_or((0, None));
    let (Some(key), None) = (video_key(query), end) else {
        return one_range().await;
    };
    let first = match video_blocks::read(key.clone(), start, None).await {
        Ok(chunk) if !chunk.bytes.is_empty() => chunk,
        _ => return one_range().await,
    };
    let Some(total) = first.total else {
        return one_range().await;
    };
    let content_type = first.content_type.clone();
    let next = first.start + first.bytes.len() as u64;
    let rest = stream::try_unfold(next, move |at| {
        let key = key.clone();
        async move {
            if at >= total {
                return Ok(None);
            }
            let chunk = video_blocks::read(key, at, None)
                .await
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            if chunk.bytes.is_empty() {
                return Ok(None);
            }
            let after = at + chunk.bytes.len() as u64;
            Ok(Some((Bytes::from(chunk.bytes), after)))
        }
    });
    let body =
        stream::once(async move { Ok::<_, std::io::Error>(Bytes::from(first.bytes)) }).chain(rest);
    (
        StatusCode::PARTIAL_CONTENT,
        [
            (header::CONTENT_TYPE, content_type),
            (header::ACCEPT_RANGES, "bytes".to_string()),
            (
                header::CONTENT_RANGE,
                format!("bytes {start}-{}/{total}", total - 1),
            ),
            (header::CONTENT_LENGTH, (total - start).to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        Body::from_stream(body),
    )
        .into_response()
}

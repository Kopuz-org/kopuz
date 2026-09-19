//! Android's artwork transport. Wry serializes custom-protocol requests under
//! one global lock shared with UI events and aborts if a response takes too
//! long. Loopback HTTP lets the WebView load covers without holding that lock.

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    extract::State,
    response::{IntoResponse, Response},
    routing::get,
};
use http::{StatusCode, header};
use tokio::sync::Semaphore;

struct ArtworkServer {
    api: Arc<dyn api::ArtworkApi>,
    downloads: Semaphore,
    timeout: Duration,
}

#[cfg(target_os = "android")]
pub async fn start(api: Arc<dyn api::ArtworkApi>) -> std::io::Result<String> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    // Only this process knows the route. Other apps and browser pages must not
    // be able to enumerate a user's private library through the loopback port.
    let path = format!("/{}/api", uuid::Uuid::new_v4());
    let endpoint = format!("http://{}{path}", listener.local_addr()?);
    let router = router(api, &path, Duration::from_secs(20));
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            tracing::error!(%error, "artwork server stopped");
        }
    });
    Ok(endpoint)
}

fn router(api: Arc<dyn api::ArtworkApi>, path: &str, timeout: Duration) -> Router {
    Router::new()
        .route(path, get(artwork))
        .with_state(Arc::new(ArtworkServer {
            api,
            downloads: Semaphore::new(8),
            timeout,
        }))
}

async fn artwork(State(server): State<Arc<ArtworkServer>>, uri: http::Uri) -> Response {
    let Some(request) = super::artwork_protocol::entity_request(&uri) else {
        return failure(StatusCode::BAD_REQUEST);
    };
    let fetch = async {
        let _permit = server
            .downloads
            .acquire()
            .await
            .map_err(|_| api::ApiError::internal("artwork server stopped"))?;
        server.api.artwork(request).await
    };
    match tokio::time::timeout(server.timeout, fetch).await {
        Ok(Ok(data)) => (
            [
                (header::CONTENT_TYPE, data.content_type.as_str()),
                (
                    header::CACHE_CONTROL,
                    "private, max-age=31536000, immutable",
                ),
                (
                    header::ACCESS_CONTROL_ALLOW_ORIGIN,
                    "https://dioxus.index.html",
                ),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            data.bytes,
        )
            .into_response(),
        Ok(Err(error)) => {
            tracing::debug!(%error, "no artwork for entity");
            failure(StatusCode::NOT_FOUND)
        }
        Err(_) => {
            tracing::debug!("artwork download timed out");
            failure(StatusCode::GATEWAY_TIMEOUT)
        }
    }
}

fn failure(status: StatusCode) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")]).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use http::Request;
    use tokio::sync::Notify;
    use tower::ServiceExt;

    #[derive(Default)]
    struct Artwork {
        slow_started: Notify,
        finish_slow: Notify,
    }

    #[async_trait::async_trait]
    impl api::ArtworkApi for Artwork {
        async fn artwork(
            &self,
            request: api::ArtworkRequest,
        ) -> Result<api::ArtworkData, api::ApiError> {
            match request.target.id() {
                "slow" => {
                    self.slow_started.notify_one();
                    self.finish_slow.notified().await;
                }
                "missing" => return Err(api::ApiError::not_found("missing cover")),
                _ => {}
            }
            Ok(api::ArtworkData {
                content_type: "image/png".into(),
                bytes: format!(
                    "{}:{}:{}",
                    request.target.kind(),
                    request.target.id(),
                    request.hq
                )
                .into_bytes(),
            })
        }

        async fn artwork_settings(&self) -> Result<Vec<api::FieldSpec>, api::ApiError> {
            Ok(Vec::new())
        }

        async fn set_artwork_settings(
            &self,
            _values: Vec<api::FieldValue>,
        ) -> Result<Vec<api::FieldSpec>, api::ApiError> {
            Ok(Vec::new())
        }
    }

    fn request(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn routes_only_entity_requests_on_the_private_path() {
        let app = router(
            Arc::new(Artwork::default()),
            "/secret/api",
            Duration::from_secs(1),
        );
        for (uri, status) in [
            ("/api?track=a", StatusCode::NOT_FOUND),
            ("/other/api?track=a", StatusCode::NOT_FOUND),
            ("/secret/local?p=/private/file", StatusCode::NOT_FOUND),
            ("/secret/api?p=/private/file", StatusCode::BAD_REQUEST),
            ("/secret/api?track=missing", StatusCode::NOT_FOUND),
        ] {
            let response = app.clone().oneshot(request(uri)).await.unwrap();
            assert_eq!(response.status(), status, "{uri}");
        }
        let response = app
            .oneshot(request("/secret/api?album=source%3Aa%26b%20%2Bc&hq=1&v=42"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        assert!(
            response.headers()[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("immutable")
        );
        let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(&bytes[..], b"album:source:a&b +c:true");
    }

    #[tokio::test]
    async fn a_slow_cover_does_not_block_other_requests() {
        let api = Arc::new(Artwork::default());
        let app = router(api.clone(), "/secret/api", Duration::from_secs(5));
        let slow = tokio::spawn(app.clone().oneshot(request("/secret/api?track=slow")));
        api.slow_started.notified().await;
        let fast = tokio::time::timeout(
            Duration::from_secs(1),
            app.oneshot(request("/secret/api?artist=fast")),
        )
        .await
        .expect("unrelated artwork must not wait for the slow download")
        .unwrap();
        assert_eq!(fast.status(), StatusCode::OK);
        assert!(!slow.is_finished());
        api.finish_slow.notify_one();
        assert_eq!(slow.await.unwrap().unwrap().status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn timeouts_release_download_slots_and_do_not_poison_later_requests() {
        let app = router(
            Arc::new(Artwork::default()),
            "/secret/api",
            Duration::from_millis(20),
        );
        // More than one full batch exercises permit release after cancellation.
        for _ in 0..10 {
            let response = app
                .clone()
                .oneshot(request("/secret/api?track=slow"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        let response = app
            .oneshot(request("/secret/api?track=fast"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

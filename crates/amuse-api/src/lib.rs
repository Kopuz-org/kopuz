static PORT: i32 = 9863;
static HOST: &str = "127.0.0.1";

use axum::response::Json;
use axum::{Router, routing::get};
use serde::Serialize;
use std::sync::{Mutex, OnceLock};
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;

#[allow(non_snake_case)]
#[derive(Debug, Clone, Serialize)]
pub struct Player {
    pub hasSong: bool,
    pub isPaused: bool,
    pub seekbarCurrentPosition: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Track {
    pub duration: u64,
    pub title: String,
    pub author: String,
    pub album: String,
    pub cover: String,
    pub url: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct AmuseApi {
    pub player: Player,
    pub track: Track,
}

static NOW_PLAYING: OnceLock<Mutex<AmuseApi>> = OnceLock::new();

impl AmuseApi {
    pub fn start() -> Result<(), Box<dyn std::error::Error>> {
        tracing::info!("Starting Amuse API");
        let app = Self::create_routes();
        tracing::info!("Routes created");
        std::thread::spawn(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let listener = tokio::net::TcpListener::bind(&format!("{}:{}", HOST, PORT))
                    .await
                    .expect("failed to bind");

                axum::serve(listener, app).await.expect("failed to serve");

                tracing::info!("Serving now");
            });
        });

        Ok(())
    }

    pub fn default_value() -> Self {
        Self {
            player: Player {
                hasSong: false,
                isPaused: false,
                seekbarCurrentPosition: 0,
            },
            track: Track {
                duration: 0,
                title: "".to_string(),
                author: "".to_string(),
                album: "".to_string(),
                cover: "".to_string(),
                url: "".to_string(),
            },
        }
    }

    fn now_playing() -> &'static Mutex<Self> {
        NOW_PLAYING.get_or_init(|| Mutex::new(Self::default_value()))
    }

    async fn query() -> Json<Self> {
        Json(Self::now_playing().lock().unwrap().clone())
    }

    async fn root() -> &'static str {
        "Amuse API server is running. GET /query to get song info."
    }

    fn create_routes() -> Router {
        let now_playing = Router::new().route("/", get(Self::query));

        Router::new()
            .route("/", get(Self::root))
            .nest("/query", now_playing.clone())
            .nest("/api", now_playing)
            .layer(
                ServiceBuilder::new()
                    .layer(CorsLayer::permissive())
                    .into_inner(),
            )
    }

    pub fn set(&self, player: Player, track: Track) {
        *Self::now_playing().lock().unwrap() = Self { player, track };
    }
}

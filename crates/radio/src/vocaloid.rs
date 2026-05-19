use crate::{RadioMetadata, RadioMetadataProvider};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

#[derive(Deserialize, Debug)]
struct VocaloidResponse {
    #[serde(rename = "Primary")]
    primary: Option<VocaloidPrimary>,
}

const STATION_NAME: &str = "Vocaloid";

#[derive(Deserialize, Debug)]
struct VocaloidPrimary {
    #[serde(rename = "Title")]
    title: Option<String>,
    #[serde(rename = "Subtitle")]
    subtitle: Option<String>,
    #[serde(rename = "Image")]
    image: Option<String>,
}

pub struct VocaloidProvider;

impl RadioMetadataProvider for VocaloidProvider {
    fn start(&self, _stream_id: &str) -> mpsc::UnboundedReceiver<RadioMetadata> {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .user_agent(concat!("Kopuz/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());

            let req = client
                .get("https://feed.platform.prod.us-west-2.tunein.com/profiles/s221579/nowPlaying")
                .send()
                .await;

            if let Ok(resp) = req {
                if let Ok(json) = resp.json::<VocaloidResponse>().await {
                    if let Some(primary) = json.primary {
                        let meta = RadioMetadata {
                            station: STATION_NAME.to_string(),
                            title: primary.title.unwrap_or_default(),
                            artist: primary.subtitle.unwrap_or_default(),
                            cover_url: primary.image,
                        };
                        if tx.send(meta).is_err() {
                            tracing::warn!("[radio] VocaloidProvider tx send error!");
                        }
                    }
                }
            }
        });
        rx
    }
}

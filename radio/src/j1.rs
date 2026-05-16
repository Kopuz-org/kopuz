use crate::{RadioMetadata, RadioMetadataProvider};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

#[derive(Deserialize, Debug)]
struct J1Response {
    station: Vec<J1Station>,
}

const STATION_NAME: &str = "J1FM";

#[derive(Deserialize, Debug)]
struct J1Station {
    name: String,
    title: String,
    artist: String,
    image_url: String,
}

pub struct J1Provider;

impl RadioMetadataProvider for J1Provider {
    fn start(&self, stream_id: &str) -> mpsc::UnboundedReceiver<RadioMetadata> {
        let (tx, rx) = mpsc::unbounded_channel();
        let stream_id = stream_id.to_string(); // "J1HITS" or "J1GOLD"

        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .user_agent("Kopuz/0.5.5")
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());

            let mut last_title = String::new();

            loop {
                if tx.is_closed() {
                    tracing::info!("[radio] J1Provider tx is closed! Exiting loop.");
                    break;
                }

                let req = client.get("https://json.j1fm.tokyo/whatweplay.json")
                    .header("Accept", "*/*")
                    .header("Accept-Language", "en-US,en;q=0.9")
                    .header("Origin", "https://www.j1fm.tokyo")
                    .header("Referer", "https://www.j1fm.tokyo/")
                    .send()
                    .await;

                if let Ok(resp) = req {
                    if let Ok(json) = resp.json::<J1Response>().await {
                        let target_name = if stream_id == "J1HITS" { "J1 HITS" } else { "J1 GOLD" };
                        if let Some(station_data) = json.station.iter().find(|s| s.name == target_name) {
                            if station_data.title != last_title {
                                last_title = station_data.title.clone();
                                let meta = RadioMetadata {
                                    station: STATION_NAME.to_string(),
                                    title: station_data.title.clone(),
                                    artist: station_data.artist.clone(),
                                    cover_url: Some(station_data.image_url.clone()),
                                };
                                if tx.send(meta).is_err() {
                                    tracing::warn!("[radio] J1Provider tx send error! Exiting loop.");
                                    break;
                                }
                            }
                        }
                    }
                }

                sleep(Duration::from_secs(2)).await;
            }
        });

        rx
    }
}

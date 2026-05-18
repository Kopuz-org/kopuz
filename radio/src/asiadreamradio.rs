use crate::{RadioMetadata, RadioMetadataProvider};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};

const STATION_NAME: &str = "Asia DREAM Radio";

pub struct AsiaDreamRadioProvider;

struct Channel {
    api_base: &'static str,
    server_id: u32,
}

#[derive(Deserialize, Debug)]
struct HistoryResponse {
    results: Vec<HistoryEntry>,
}

#[derive(Deserialize, Debug)]
struct HistoryEntry {
    ts: Option<i64>,
    title: Option<String>,
    author: Option<String>,
    metadata: Option<String>,
    img_large_url: Option<String>,
    img_medium_url: Option<String>,
    img_url: Option<String>,
}

fn split_metadata(s: &str) -> (&str, &str) {
    match s.split_once(" - ") {
        Some((a, t)) => (a.trim(), t.trim()),
        None => ("", s.trim()),
    }
}

fn channel_for(stream_id: &str) -> Option<Channel> {
    Some(match stream_id {
        "japan_hits" => Channel {
            api_base: "https://quincy.torontocast.com:1970",
            server_id: 1,
        },
        "natsukashii" => Channel {
            api_base: "https://quincy.torontocast.com:1970",
            server_id: 3,
        },
        "jpop_kawaii" => Channel {
            api_base: "https://kathy.torontocast.com:2650",
            server_id: 2,
        },
        "jpop_power" => Channel {
            api_base: "https://kathy.torontocast.com:2650",
            server_id: 1,
        },
        "jazz_sakura" => Channel {
            api_base: "https://kathy.torontocast.com:3310",
            server_id: 1,
        },
        "jrock" => Channel {
            api_base: "https://kathy.torontocast.com:3310",
            server_id: 4,
        },
        "jclub_hiphop" => Channel {
            api_base: "https://kathy.torontocast.com:3310",
            server_id: 5,
        },
        "bandstand_jazz" => Channel {
            api_base: "https://cast1.torontocast.com:2050",
            server_id: 1,
        },
        _ => return None,
    })
}

impl RadioMetadataProvider for AsiaDreamRadioProvider {
    fn start(&self, stream_id: &str) -> mpsc::UnboundedReceiver<RadioMetadata> {
        let (tx, rx) = mpsc::unbounded_channel();
        let stream_id = stream_id.to_string();

        tokio::spawn(async move {
            let Some(channel) = channel_for(&stream_id) else {
                tracing::warn!("[radio] AsiaDreamRadio: unknown stream_id {stream_id}");
                return;
            };

            let url = format!(
                "{}/api/v2/history/?limit=1&offset=0&server={}",
                channel.api_base, channel.server_id
            );

            let client = reqwest::Client::builder()
                .user_agent(concat!("Kopuz/", env!("CARGO_PKG_VERSION")))
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());

            let mut last_ts: Option<i64> = None;

            loop {
                if tx.is_closed() {
                    break;
                }

                let req = client.get(&url).send().await;

                if let Ok(resp) = req {
                    if let Ok(json) = resp.json::<HistoryResponse>().await {
                        if let Some(entry) = json.results.into_iter().next() {
                            if entry.ts.is_some() && entry.ts == last_ts {
                                sleep(Duration::from_secs(10)).await;
                                continue;
                            }
                            last_ts = entry.ts;

                            let title_raw = entry.title.as_deref().map(str::trim).unwrap_or("");
                            let author_raw = entry.author.as_deref().map(str::trim).unwrap_or("");
                            let (artist, title) = if !title_raw.is_empty() && !author_raw.is_empty()
                            {
                                (author_raw.to_string(), title_raw.to_string())
                            } else {
                                let (mut a, mut t) = (author_raw, title_raw);
                                let split;
                                if let Some(meta) = entry.metadata.as_deref() {
                                    split = split_metadata(meta);
                                    if a.is_empty() {
                                        a = split.0;
                                    }
                                    if t.is_empty() {
                                        t = split.1;
                                    }
                                }
                                (
                                    if a.is_empty() {
                                        "Unknown Artist".to_string()
                                    } else {
                                        a.to_string()
                                    },
                                    if t.is_empty() {
                                        "Unknown".to_string()
                                    } else {
                                        t.to_string()
                                    },
                                )
                            };
                            let cover_url = entry
                                .img_large_url
                                .or(entry.img_medium_url)
                                .or(entry.img_url);
                            let meta = RadioMetadata {
                                station: STATION_NAME.to_string(),
                                title,
                                artist,
                                cover_url,
                            };
                            if tx.send(meta).is_err() {
                                break;
                            }
                        }
                    }
                }

                sleep(Duration::from_secs(10)).await;
            }
        });

        rx
    }
}

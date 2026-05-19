use tokio::sync::mpsc;
pub mod stations;
#[cfg(not(target_arch = "wasm32"))]
pub mod listen_moe;
#[cfg(not(target_arch = "wasm32"))]
pub mod j1;
#[cfg(not(target_arch = "wasm32"))]
pub mod doujinstyle;
#[cfg(not(target_arch = "wasm32"))]
pub mod vocaloid;
#[cfg(not(target_arch = "wasm32"))]
pub mod asiadreamradio;

#[derive(Debug, Clone, PartialEq)]
pub struct RadioMetadata {
    pub station: String,
    pub title: String,
    pub artist: String,
    pub cover_url: Option<String>,
}

pub trait RadioMetadataProvider: Send + Sync {
    fn start(&self, stream_id: &str) -> mpsc::UnboundedReceiver<RadioMetadata>;
}

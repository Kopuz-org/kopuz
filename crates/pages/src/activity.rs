use config::{AppConfig, MusicSource};
use dioxus::prelude::*;

use crate::local::activity::LocalLogs;
use crate::server::activity::ServerLogs;

#[component]
pub fn Activity(config: Signal<AppConfig>) -> Element {
    let is_server = config.read().active_source == MusicSource::Server;

    rsx! {
        if is_server {
            ServerLogs { config }
        } else {
            LocalLogs { config }
        }
    }
}

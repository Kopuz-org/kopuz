use config::{AppConfig, MusicSource};
use dioxus::prelude::*;
use reader::Library;

use crate::local::logs::LocalLogs;
use crate::server::logs::ServerLogs;

#[component]
pub fn Logs(library: Signal<Library>, config: Signal<AppConfig>) -> Element {
    let is_server = config.read().active_source == MusicSource::Server;

    rsx! {
        if is_server {
            ServerLogs { library, config }
        } else {
            LocalLogs { library, config }
        }
    }
}

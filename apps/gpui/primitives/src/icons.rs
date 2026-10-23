//! Icon paths into the bundled Lucide set; `gpui_kit::assets::AllAssets` serves them.

use gpui_kit::component::Icon;

pub const PLAY: &str = "icons/play.svg";
pub const PAUSE: &str = "icons/pause.svg";
pub const SKIP_BACK: &str = "icons/skip-back.svg";
pub const SKIP_FORWARD: &str = "icons/skip-forward.svg";
pub const SHUFFLE: &str = "icons/shuffle.svg";
pub const REPEAT: &str = "icons/repeat.svg";
pub const REPEAT_ONE: &str = "icons/repeat-1.svg";
pub const VOLUME: &str = "icons/volume-2.svg";
pub const VOLUME_LOW: &str = "icons/volume-1.svg";
pub const VOLUME_MUTED: &str = "icons/volume-x.svg";
pub const HEART: &str = "icons/heart.svg";
pub const HEART_OFF: &str = "icons/heart-off.svg";
pub const MUSIC: &str = "icons/music.svg";
pub const DISC: &str = "icons/disc-3.svg";
pub const MIC: &str = "icons/mic-vocal.svg";
pub const RADIO: &str = "icons/radio.svg";
pub const LIST: &str = "icons/list-music.svg";
pub const LIBRARY: &str = "icons/library.svg";
pub const HOME: &str = "icons/house.svg";
pub const SEARCH: &str = "icons/search.svg";
pub const COMPASS: &str = "icons/compass.svg";
pub const CLOCK: &str = "icons/clock.svg";
pub const DOWNLOAD: &str = "icons/download.svg";
pub const SETTINGS: &str = "icons/settings.svg";
pub const PLUS: &str = "icons/plus.svg";
pub const TRASH: &str = "icons/trash.svg";
pub const MORE: &str = "icons/ellipsis.svg";
pub const CHEVRON_LEFT: &str = "icons/chevron-left.svg";
pub const CHEVRON_RIGHT: &str = "icons/chevron-right.svg";
pub const CHEVRON_DOWN: &str = "icons/chevron-down.svg";
pub const CHEVRON_UP: &str = "icons/chevron-up.svg";
pub const CLOSE: &str = "icons/x.svg";
pub const CHECK: &str = "icons/check.svg";
pub const SHARE: &str = "icons/share-2.svg";
pub const QUEUE_ADD: &str = "icons/list-plus.svg";
pub const QUEUE_NEXT: &str = "icons/list-start.svg";
pub const FOLDER: &str = "icons/folder.svg";
pub const EDIT: &str = "icons/pencil.svg";
pub const EXPAND: &str = "icons/maximize-2.svg";
pub const COLLAPSE: &str = "icons/minimize-2.svg";
pub const GRID: &str = "icons/layout-grid.svg";
pub const ROWS: &str = "icons/rows-3.svg";
pub const SORT: &str = "icons/arrow-up-down.svg";
pub const REFRESH: &str = "icons/refresh-cw.svg";
pub const THUMBS_DOWN: &str = "icons/thumbs-down.svg";
pub const SPEAKER: &str = "icons/speaker.svg";
pub const LYRICS: &str = "icons/mic.svg";
pub const ACTIVITY: &str = "icons/flame.svg";
pub const WARNING: &str = "icons/triangle-alert.svg";
pub const LOCK: &str = "icons/lock.svg";
pub const COPY: &str = "icons/copy.svg";
pub const SERVER: &str = "icons/server.svg";
pub const CLOUD: &str = "icons/cloud.svg";
pub const HARD_DRIVE: &str = "icons/hard-drive.svg";
pub const WIFI_OFF: &str = "icons/wifi-off.svg";
pub const CLOUD_OFF: &str = "icons/cloud-off.svg";
pub const WIFI: &str = "icons/wifi.svg";
pub const FOLDER_TREE: &str = "icons/folder-tree.svg";

/// An icon by path, e.g. `icon(icons::PLAY)`; the active skin may swap what it draws.
pub fn icon(path: &'static str) -> Icon {
    match crate::skin::asset(path) {
        Some(swapped) => Icon::default().path(swapped),
        None => Icon::default().path(path),
    }
}

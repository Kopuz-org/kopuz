//! App-wide mirrors of daemon state, each a GPUI entity reachable through `Model::global(cx)`.

pub mod actions;
pub mod ambient;
mod artwork;
mod artwork_edit;
mod backdrop;
mod catalog_cache;
mod chrome;
mod config;
mod downloads;
mod favorites;
mod for_you;
mod freeze;
mod invalidation;
mod jobs;
pub mod layout;
mod nav;
mod notices;
mod opener;
pub mod paging;
pub mod palette;
mod picture;
mod player;
mod playing;
mod playlists;
mod query;
mod queue;
mod settings_filter;
mod setup;
mod signin;
mod sources;
mod target;
mod updates;

use gpui_kit::{App, AppContext as _, Entity, Global};

pub use artwork::{Arrival, ArtworkBytes, ArtworkKey, Residents, artwork_source};
pub use artwork_edit::{content_type_for, pick_and_upload, pick_image, remove_artwork};
pub use backdrop::{blurred_backdrop, resolve_backdrop};
pub use catalog_cache::{Cached, CatalogCache};
pub use chrome::{Chrome, SidePanel, register_mini_player, toggle_mini_player};
pub use config::{ConfigModel, lock_cause};
pub use downloads::{BatchFinished, DownloadItem, DownloadsModel, ItemState, Offline};
pub use favorites::FavoritesModel;
pub use for_you::{ForYouModel, ForYouState};
pub use freeze::Freeze;
pub use invalidation::{Invalidation, TablesChanged};
pub use jobs::{Finished, JobEvent, JobsModel, kind_key, kind_label};
pub use layout::{Layout, LayoutModel};
pub use link::{Link, api};
pub use nav::{Navigator, Route};
pub use notices::{DAEMON_GONE, Notice, Notices, Record, Retracted, Undo};
pub use opener::{
    BATCH_METADATA, FOLDER_PICKER, Opener, RENAME, Rename, SHORTCUTS, TRACK_METADATA, URL_DOWNLOAD,
};
pub use paging::Pager;
pub use palette::Swatches;
pub use picture::{PictureError, Pictured, copy_picture, extension, file_name, save_picture};
pub use player::{PlayerModel, Playhead};
pub use playing::Playing;
pub use playlists::PlaylistsModel;
pub use query::{FirstRead, Query};
pub use queue::{QueueModel, QueueSummary};
pub use settings_filter::SettingsFilter;
pub use setup::{Decision, SetupModel};
pub use signin::{
    SignInWait, browser as browser_sign_in, register_password_form, register_session_chooser,
    sign_in_again, sign_in_wait, start_sign_in,
};
pub use sources::{NetworkNotice, SourceStatus, SourcesModel, SwitchStarted};
pub use target::{Kind, RowEdit, RowScope, Target, TrackActions};
pub use updates::UpdateModel;

/// `cx.notify()`, counted: a shared model's notify dirties every ancestor, so the profiler wants it.
pub(crate) fn notify<T: 'static>(cx: &mut gpui_kit::Context<T>) {
    profile::notifies::record();
    cx.notify();
}

/// How the app is meant to look, for a view that only reads it.
pub fn layout(cx: &App) -> &Layout {
    LayoutModel::global(cx).read(cx).get()
}

/// Every row of a listing, the way the Dioxus frontend asks for "all".
pub fn all() -> api::Page {
    api::Page {
        offset: 0,
        limit: u32::MAX,
    }
}

/// The daemon's own sentence for a failed call; `ApiError`'s Display opens with a Rust enum variant.
pub fn human(error: &api::ApiError) -> gpui_kit::SharedString {
    match error.code {
        // The one failure the message cannot explain, because there is nobody to write it.
        api::ErrorCode::DaemonGone => gpui_kit::SharedString::from(i18n::t("daemon_unreachable")),
        _ => said(error.code, &error.message, error.reason),
    }
}

/// A failure the daemon reported inside a message, said the same way as [`human`].
pub fn human_body(error: &api::ErrorBody) -> gpui_kit::SharedString {
    said(error.code, &error.message, error.reason)
}

fn said(
    code: api::ErrorCode,
    message: &str,
    reason: Option<api::ErrorReason>,
) -> gpui_kit::SharedString {
    match reason {
        Some(reason) => gpui_kit::SharedString::from(i18n::t(reason.message_key())),
        None => gpui_kit::SharedString::from(i18n::t_with(
            code.message_key(),
            &[("detail", message.to_string().into())],
        )),
    }
}

struct Handle<T>(Entity<T>);

impl<T: 'static> Global for Handle<T> {}

fn handle<T: 'static>(cx: &App) -> Entity<T> {
    cx.global::<Handle<T>>().0.clone()
}

fn register<T: 'static>(cx: &mut App, build: impl FnOnce(&mut gpui_kit::Context<T>) -> T) {
    let entity = cx.new(build);
    cx.set_global(Handle(entity));
}

/// Create every model; call once after `link::init`, before opening a window.
pub fn init(cx: &mut App) {
    register(cx, LayoutModel::new);
    register(cx, Notices::new);
    register(cx, Invalidation::new);
    register(cx, ConfigModel::new);
    register(cx, SourcesModel::new);
    register(cx, PlayerModel::new);
    register(cx, Playhead::new);
    register(cx, QueueModel::new);
    register(cx, FavoritesModel::new);
    register(cx, PlaylistsModel::new);
    register(cx, ForYouModel::new);
    register(cx, JobsModel::new);
    register(cx, DownloadsModel::new);
    register(cx, Navigator::new);
    register(cx, Chrome::new);
    register(cx, SettingsFilter::new);
    register(cx, SetupModel::new);
    register(cx, UpdateModel::new);
}

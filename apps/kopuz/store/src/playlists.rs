use api::{PlaylistCatalog, PlaylistFolderInfo, PlaylistInfo, Table};
use gpui_kit::{App, Context, Entity, Subscription, Task};
use link::Link;

use crate::invalidation::{Invalidation, TablesChanged};
use crate::notices::Notices;

/// The playlist catalog, shared by the sidebar, pickers and the playlists page.
pub struct PlaylistsModel {
    catalog: PlaylistCatalog,
    loaded: bool,
    _fetch: Option<Task<()>>,
    _subscription: Subscription,
}

impl PlaylistsModel {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let invalidation = Invalidation::global(cx);
        let subscription = cx.subscribe(&invalidation, |this, _, event: &TablesChanged, cx| {
            if event.touches(&[Table::Playlists, Table::Folders]) {
                this.refetch(cx);
            }
        });
        Self {
            catalog: PlaylistCatalog::default(),
            loaded: false,
            _fetch: None,
            _subscription: subscription,
        }
    }

    pub fn refetch(&mut self, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.playlists().await });
        self._fetch = Some(cx.spawn(async move |this, cx| {
            let Ok(catalog) = call.await else { return };
            this.update(cx, |this, cx| {
                this.catalog = catalog;
                this.loaded = true;
                crate::notify(cx);
            })
            .ok();
        }));
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn playlists(&self) -> &[PlaylistInfo] {
        &self.catalog.playlists
    }

    pub fn folders(&self) -> &[PlaylistFolderInfo] {
        &self.catalog.folders
    }

    pub fn playlist(&self, id: &str) -> Option<&PlaylistInfo> {
        self.catalog
            .playlists
            .iter()
            .find(|playlist| playlist.id == id)
    }

    pub fn add_tracks(&mut self, id: String, keys: Vec<String>, cx: &mut Context<Self>) {
        let call = Link::global(cx)
            .call(move |api| async move { api.add_playlist_tracks(id, keys).await });
        cx.spawn(async move |_, cx| {
            if let Err(error) = call.await {
                cx.update(|cx| Notices::failed(&error, cx));
            }
        })
        .detach();
    }

    pub fn create(&mut self, name: String, keys: Vec<String>, cx: &mut Context<Self>) {
        let call =
            Link::global(cx).call(move |api| async move { api.create_playlist(name, keys).await });
        cx.spawn(async move |_, cx| {
            if let Err(error) = call.await {
                cx.update(|cx| Notices::failed(&error, cx));
            }
        })
        .detach();
    }
}

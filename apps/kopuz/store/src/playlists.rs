use api::{PlaylistCatalog, PlaylistFolderInfo, PlaylistInfo, Table};
use gpui_kit::{App, Context, Entity, Subscription, Task};
use link::Link;

use crate::invalidation::{Invalidation, TablesChanged};
use crate::notices::Notices;

/// The playlist catalog, shared by the sidebar, pickers and the playlists page.
pub struct PlaylistsModel {
    catalog: PlaylistCatalog,
    loaded: bool,
    /// Why the last read failed; cleared by the next one that lands.
    error: Option<api::ApiError>,
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
            error: None,
            _fetch: None,
            _subscription: subscription,
        }
    }

    pub fn refetch(&mut self, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.playlists().await });
        self._fetch = Some(cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(catalog) => {
                        this.catalog = catalog;
                        this.loaded = true;
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                crate::notify(cx);
            })
            .ok();
        }));
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// The last read failed, and this is why.
    pub fn error(&self) -> Option<&api::ApiError> {
        self.error.as_ref()
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

    /// Add `keys` the playlist does not hold yet, and say how many went in.
    pub fn add_tracks(&mut self, id: String, keys: Vec<String>, cx: &mut Context<Self>) {
        let (name, keys) = match self.playlist(&id) {
            Some(playlist) => (
                playlist.name.clone(),
                keys.into_iter()
                    .filter(|key| !playlist.track_keys.contains(key))
                    .collect::<Vec<_>>(),
            ),
            None => (String::new(), keys),
        };
        if keys.is_empty() {
            Notices::info(
                i18n::t_with("playlist_already_holds", &[("playlist", name.into())]),
                cx,
            );
            return;
        }
        let count = keys.len();
        let call = Link::global(cx)
            .call(move |api| async move { api.add_playlist_tracks(id, keys).await });
        cx.spawn(async move |_, cx| {
            let result = call.await;
            cx.update(|cx| match result {
                Ok(_) => Notices::info(
                    i18n::t_with(
                        "playlist_tracks_added",
                        &[("count", count.into()), ("playlist", name.into())],
                    ),
                    cx,
                ),
                Err(error) => Notices::failed(&error, cx),
            });
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

use std::collections::HashSet;

use api::Table;
use gpui_kit::{App, Context, Entity, Subscription, Task};
use link::Link;

use crate::invalidation::{Invalidation, TablesChanged};
use crate::notices::Notices;

/// Favorited track keys, set optimistically and reverted if the daemon refuses.
pub struct FavoritesModel {
    keys: HashSet<String>,
    loaded: bool,
    _fetch: Option<Task<()>>,
    _subscription: Subscription,
}

impl FavoritesModel {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let invalidation = Invalidation::global(cx);
        let subscription = cx.subscribe(&invalidation, |this, _, event: &TablesChanged, cx| {
            if event.touches(&[Table::Favorites]) {
                this.refetch(cx);
            }
        });
        Self {
            keys: HashSet::new(),
            loaded: false,
            _fetch: None,
            _subscription: subscription,
        }
    }

    fn refetch(&mut self, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.favorites().await });
        self._fetch = Some(cx.spawn(async move |this, cx| {
            let Ok(view) = call.await else { return };
            this.update(cx, |this, cx| {
                this.keys = view.refs.into_iter().collect();
                this.loaded = true;
                crate::notify(cx);
            })
            .ok();
        }));
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn is_favorite(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    /// Keys in no particular order.
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.keys.iter()
    }

    pub fn toggle(&mut self, key: String, cx: &mut Context<Self>) {
        let favorite = !self.is_favorite(&key);
        self.set(key, favorite, cx);
    }

    pub fn set(&mut self, key: String, favorite: bool, cx: &mut Context<Self>) {
        let changed = if favorite {
            self.keys.insert(key.clone())
        } else {
            self.keys.remove(&key)
        };
        if changed {
            crate::notify(cx);
        }
        let call = Link::global(cx).call({
            let key = key.clone();
            move |api| async move { api.set_favorite(key, favorite).await }
        });
        cx.spawn(async move |this, cx| {
            if let Err(error) = call.await {
                this.update(cx, |this, cx| {
                    if favorite {
                        this.keys.remove(&key);
                    } else {
                        this.keys.insert(key);
                    }
                    crate::notify(cx);
                    Notices::failed(&error, cx);
                })
                .ok();
            }
        })
        .detach();
    }
}

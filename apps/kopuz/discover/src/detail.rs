//! What the catalog opens onto: one browsed playlist, and a source's own page for an artist.

use std::rc::Rc;

use api::{CatalogDetail, CatalogDetailRequest, CatalogItemKind};
use components::format::t;
use components::{
    DetailActions, DetailHeader, EmptyState, HeaderLayout, Loading, Look, PageHeader, TrackActions,
    TrackList, TrackListEvent, TrackListOptions, icons,
};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled, Subscription, Task, Window, div,
};
use store::{Link, PlayerModel, actions};

use crate::shelves::Shelves;
use crate::states;

/// One catalog playlist opened from the feed, read-only: a browsed list never becomes a saved one.
pub struct CatalogPlaylist {
    id: SharedString,
    title: SharedString,
    list: Entity<TrackList>,
    // The rows' keys, taken once per answer rather than per frame.
    keys: Rc<[String]>,
    loading: bool,
    error: Option<SharedString>,
    // Bumped per load; a slow answer for one id must not land after the user moved to another.
    generation: u64,
    _load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl CatalogPlaylist {
    pub fn new(
        id: SharedString,
        title: SharedString,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let list = cx.new(|cx| {
            TrackList::new(cx)
                .with_options(TrackListOptions {
                    selectable: false,
                    ..TrackListOptions::default()
                })
                .with_actions(TrackActions::default())
        });
        commands::bind_track_list(&list, cx);
        let subscriptions = vec![
            cx.subscribe(&list, |_, list, event: &TrackListEvent, cx| {
                if let TrackListEvent::Play(index) = event {
                    let keys = list.read(cx).keys();
                    actions::play_tracks(keys, Some(*index as u32), cx);
                }
            }),
            // The shuffle button reads the player at render.
            cx.observe(&PlayerModel::global(cx), |_, _, cx| cx.notify()),
        ];
        let mut page = Self {
            id,
            title,
            list,
            keys: Rc::from([]),
            loading: false,
            error: None,
            generation: 0,
            _load: None,
            _subscriptions: subscriptions,
        };
        page.load(cx);
        page
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let id = self.id.to_string();
        self.loading = true;
        self.error = None;
        cx.notify();
        let call = Link::global(cx).call(move |api| async move {
            let request = CatalogDetailRequest::new(CatalogItemKind::Playlist, id.clone());
            match api.catalog_detail(request).await {
                Ok(detail) => Ok(detail),
                // The route carries no kind, so an album opened here is a second try, not an id sniff.
                Err(_) => {
                    api.catalog_detail(CatalogDetailRequest::new(CatalogItemKind::Album, id))
                        .await
                }
            }
        });
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(detail) => {
                        this.keys = detail.tracks.iter().map(|row| row.key.clone()).collect();
                        this.list
                            .update(cx, |list, cx| list.set_rows(detail.tracks, cx));
                    }
                    Err(error) => this.error = Some(states::catalog_failure(&error)),
                }
                cx.notify();
            })
            .ok();
        }));
    }
}

impl Render for CatalogPlaylist {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("catalog-playlist");
        let empty = !self.loading && self.error.is_none() && self.keys.is_empty();
        components::page()
            .pb_8()
            .child(
                PageHeader::new(self.title.clone()).when(!self.keys.is_empty(), |header| {
                    header.action(
                        DetailActions::new("catalog-playlist", self.keys.clone())
                            .look(Look::Compact)
                            .play(commands::play_all)
                            .shuffle(commands::set_shuffle),
                    )
                }),
            )
            .map(|page| {
                if self.loading {
                    page.child(Loading::new())
                } else if let Some(message) = self.error.clone() {
                    page.child(EmptyState::failed(
                        message,
                        cx.listener(|this, _, _, cx| this.load(cx)),
                    ))
                } else if empty {
                    page.child(EmptyState::new(t("playlist_not_found")).icon(icons::LIST))
                } else {
                    page.child(div().flex_1().min_h_0().child(self.list.clone()))
                }
            })
    }
}

/// A remote artist profile: the source's own page for an artist the library does not hold.
pub struct CatalogArtist {
    key: String,
    detail: Option<CatalogDetail>,
    shelves: Entity<Shelves>,
    loading: bool,
    error: Option<SharedString>,
    generation: u64,
    scroll: ScrollHandle,
    _load: Option<Task<()>>,
}

impl CatalogArtist {
    pub fn new(key: String, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            key,
            detail: None,
            shelves: cx.new(Shelves::new),
            loading: false,
            error: None,
            generation: 0,
            scroll: ScrollHandle::new(),
            _load: None,
        };
        page.load(cx);
        page
    }

    /// Blank until the daemon answers: the key carries no display name.
    fn name(&self) -> SharedString {
        self.detail
            .as_ref()
            .map(|detail| SharedString::from(detail.title.clone()))
            .unwrap_or_default()
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let request = CatalogDetailRequest::artist(&self.key);
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        self.error = None;
        cx.notify();
        let call =
            Link::global(cx).call(move |api| async move { api.catalog_detail(request).await });
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(detail) => {
                        let shelves = detail.shelves.clone();
                        this.shelves.update(cx, |held, cx| held.set(shelves, cx));
                        this.detail = Some(detail);
                    }
                    Err(error) => this.error = Some(states::catalog_failure(&error)),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn banner(&self, detail: &CatalogDetail, cx: &mut Context<Self>) -> AnyElement {
        // The artist's shuffle and mix are radios the source issues, not playlists it can list.
        let shuffle = detail.shuffle_radio.clone();
        let mix = detail.mix_radio.clone();
        let legacy = detail.playback_id.clone().filter(|_| shuffle.is_none());
        let bar = (shuffle.is_some() || mix.is_some() || legacy.is_some()).then(|| {
            let mut bar =
                DetailActions::new("catalog-artist", Rc::<[String]>::from([])).look(Look::Banner);
            if let Some(id) = shuffle {
                bar =
                    bar.shuffle_with(move |_, _, cx| actions::start_playlist_radio(id.clone(), cx));
            }
            if let Some(id) = legacy {
                bar = bar.shuffle_with(cx.listener(move |this, _, _, cx| {
                    let id = id.clone();
                    this.shelves.update(cx, |shelves, cx| {
                        shelves.play(CatalogItemKind::Playlist, id, cx)
                    });
                }));
            }
            if let Some(id) = mix {
                bar = bar.radio(move |_, _, cx| actions::start_playlist_radio(id.clone(), cx));
            }
            bar
        });
        DetailHeader::new(detail.title.clone())
            .layout(HeaderLayout::Banner)
            .artwork(detail.artwork.as_ref())
            .when_some(detail.subtitle.clone(), |header, subtitle| {
                header.meta(subtitle)
            })
            .description(detail.description.clone())
            .when_some(bar, |header, bar| header.action(bar))
            .into_any_element()
    }
}

impl Render for CatalogArtist {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("catalog-artist");
        components::scroll::smooth("catalog-artist-scroll", &self.scroll, window, cx);
        let banner = self.detail.clone().map(|detail| self.banner(&detail, cx));
        div()
            .id("catalog-artist")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                v_flex()
                    .w_full()
                    .px_10()
                    .pt_6()
                    .pb_10()
                    .gap_8()
                    .child(PageHeader::new(self.name()))
                    .when(self.loading, |page| page.child(Loading::new()))
                    .when_some(self.error.clone(), |page, message| {
                        page.child(EmptyState::failed(
                            message,
                            cx.listener(|this, _, _, cx| this.load(cx)),
                        ))
                    })
                    .children(banner)
                    .child(self.shelves.clone()),
            )
            .into_any_element()
    }
}

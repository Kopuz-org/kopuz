//! The Discover landing: the source's mixes, a row per page it declares, and the library's forgotten corners.

use std::collections::HashMap;
use std::time::Duration;

use api::{AlbumInfo, CatalogShelf, DiscoverRows, PageEntry, SourceCapabilities, TrackInfo};
use components::format::{t, text};
use components::{
    Carousel, EmptyState, PageHeader, PlayState, SearchTrigger, Tile, TriggerSize, icons, motion,
    skeleton, style,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, Context, Entity,
    InteractiveElement as _, IntoElement, Keystroke, ParentElement as _, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled, Subscription, Task, Window, div,
    pulsating_between,
};
use store::{
    Chrome, Invalidation, Link, Pictured, Route, SourcesModel, TablesChanged, Target, actions,
};

use crate::page::placeholder_bands;
use crate::rows::{self, Fetch, Fill, Row};
use crate::shelves::{GAP, Shelves, TILE};
use crate::states;

/// Enough placeholders to fill a wide window.
const PLACEHOLDERS: usize = 8;
const MIXES: &str = "mixes";

pub struct Landing {
    pages: Vec<PageEntry>,
    rows: Fetch<DiscoverRows>,
    page_rows: Vec<Fetch<Option<CatalogShelf>>>,
    // Bumped per ask; an answer from before a refresh or a source change is dropped.
    generation: u64,
    started: bool,
    // Each row's fade over its placeholder, by row name.
    reveals: HashMap<String, motion::Fade>,
    // The mixes and the page rows, which play and page as the catalog's own shelves do.
    shelves: Entity<Shelves>,
    never_played: ScrollHandle,
    forgotten: ScrollHandle,
    scroll: ScrollHandle,
    // The source's capabilities as the last frame read them, none before the list loads; frozen pages keep them.
    caps: Option<SourceCapabilities>,
    frozen: bool,
    _rows: Option<Task<()>>,
    _pages: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Landing {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let sources = SourcesModel::global(cx);
        let invalidation = Invalidation::global(cx);
        let subscriptions = vec![
            // Capabilities arrive with the source list, so the first ask follows that answer.
            cx.observe(&sources, |this, _, cx| this.follow_sources(cx)),
            cx.subscribe(&invalidation, |this, _, event: &TablesChanged, cx| {
                if event.is_all() && this.started {
                    this.ask(false, cx);
                }
            }),
        ];
        let mut landing = Self {
            pages: Vec::new(),
            rows: Fetch::Loading,
            page_rows: Vec::new(),
            generation: 0,
            started: false,
            reveals: HashMap::new(),
            shelves: cx.new(Shelves::new),
            never_played: ScrollHandle::new(),
            forgotten: ScrollHandle::new(),
            scroll: ScrollHandle::new(),
            caps: None,
            frozen: false,
            _rows: None,
            _pages: Vec::new(),
            _subscriptions: subscriptions,
        };
        landing.follow_sources(cx);
        landing
    }

    /// Stop asking and stop following the source, drawing what is held.
    pub(crate) fn freeze(&mut self, _: &mut Context<Self>) {
        self.frozen = true;
        self._subscriptions.clear();
        self._rows = None;
        self._pages.clear();
    }

    fn follow_sources(&mut self, cx: &mut Context<Self>) {
        let sources = SourcesModel::global(cx);
        if !sources.read(cx).is_loaded() {
            return;
        }
        let pages = rows::row_pages(&sources.read(cx).capabilities().pages);
        if !self.started {
            self.started = true;
            self.pages = pages;
            self.ask(false, cx);
        } else if pages != self.pages {
            self.pages = pages;
            self.ask_pages(cx);
        }
    }

    /// Ask for every row again; `fresh` has the daemon build its rows anew rather than hand back what it holds.
    fn ask(&mut self, fresh: bool, cx: &mut Context<Self>) {
        self.generation += 1;
        self.ask_rows(fresh, cx);
        self.ask_pages(cx);
    }

    fn ask_rows(&mut self, fresh: bool, cx: &mut Context<Self>) {
        let generation = self.generation;
        self.rows = Fetch::Loading;
        cx.notify();
        let call = Link::global(cx).call(move |api| async move {
            match fresh {
                true => api.refresh_discover_rows().await,
                false => api.discover_rows().await,
            }
        });
        let first = store::FirstRead::begin(cx);
        self._rows = Some(cx.spawn(async move |this, cx| {
            let result = call.await;
            let _first = first;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.rows = match result {
                    Ok(rows) => {
                        let mixes = CatalogShelf {
                            items: rows.mixes.clone(),
                            ..CatalogShelf::default()
                        };
                        this.shelves.update(cx, |shelves, cx| {
                            shelves.put_row(MIXES, t("discover_your_mixes"), mixes, None, cx)
                        });
                        Fetch::Ready(rows)
                    }
                    Err(error) => {
                        tracing::debug!(%error, "discover rows failed");
                        Fetch::Failed(error)
                    }
                };
                cx.notify();
            })
            .ok();
        }));
    }

    fn ask_pages(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        self.page_rows = vec![Fetch::Loading; self.pages.len()];
        cx.notify();
        self._pages = self
            .pages
            .iter()
            .enumerate()
            .map(|(index, page)| {
                let id = page.id.clone();
                let call = Link::global(cx).call(move |api| async move {
                    api.catalog_detail(api::CatalogDetailRequest::page(id))
                        .await
                });
                let first = store::FirstRead::begin(cx);
                cx.spawn(async move |this, cx| {
                    let result = call.await;
                    let _first = first;
                    this.update(cx, |this, cx| {
                        if this.generation != generation {
                            return;
                        }
                        let fetched = match result {
                            Ok(detail) => Fetch::Ready(rows::first_shelf(&detail.shelves)),
                            Err(error) => {
                                tracing::debug!(%error, "discover page row failed");
                                Fetch::Failed(error)
                            }
                        };
                        if let (Fetch::Ready(Some(shelf)), Some(page)) =
                            (&fetched, this.pages.get(index))
                        {
                            let title = text(&page.label);
                            let see_all = Route::CatalogPage {
                                id: page.id.clone().into(),
                            };
                            let shelf = shelf.clone();
                            let key = row_key(page);
                            this.shelves.update(cx, |shelves, cx| {
                                shelves.put_row(key, title, shelf, Some(see_all), cx)
                            });
                        }
                        if let Some(slot) = this.page_rows.get_mut(index) {
                            *slot = fetched;
                        }
                        cx.notify();
                    })
                    .ok();
                })
            })
            .collect();
    }

    fn title(&self, row: Row) -> SharedString {
        match row {
            Row::Mixes => t("discover_your_mixes"),
            Row::Page(index) => self
                .pages
                .get(index)
                .map(|page| text(&page.label))
                .unwrap_or_default(),
            Row::NeverPlayed => t("discover_never_played"),
            Row::ForgottenFavorites => t("discover_forgotten_favorites"),
        }
    }

    fn row(
        &mut self,
        row: Row,
        fill: Fill,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = row_name(row, &self.pages);
        // Kept per answer on the view, so a revisit finds every row already in rather than fading it again.
        let fade = self.reveals.entry(name.clone()).or_default();
        if fill == Fill::Placeholder {
            fade.snap(false);
            return placeholder_row(&name, self.title(row));
        }
        let shown = fade.sample(true, motion::MEDIUM, window, cx);
        let content = match (row, &self.rows) {
            (Row::Mixes, _) => self
                .shelves
                .update(cx, |shelves, cx| shelves.row(MIXES, cx)),
            (Row::Page(index), _) => self.pages.get(index).and_then(|page| {
                let key = row_key(page);
                self.shelves.update(cx, |shelves, cx| shelves.row(&key, cx))
            }),
            (Row::NeverPlayed, Fetch::Ready(rows)) => Some(albums_row(
                self.title(row),
                &rows.never_played,
                &self.never_played,
            )),
            (Row::ForgottenFavorites, Fetch::Ready(rows)) => Some(tracks_row(
                self.title(row),
                &rows.forgotten_favorites,
                &self.forgotten,
            )),
            _ => None,
        };
        // The content fades over where its placeholder stood, once per answer.
        let placeholder = (shown < 1.).then(|| placeholder_row(&name, self.title(row)));
        skeleton::reveal(div().w_full().children(content), placeholder, shown)
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shortcut = commands::keystroke_for("app.palette", cx)
            .and_then(|held| Keystroke::parse(&held).ok());
        let search = SearchTrigger::new("discover-search-hint", t("discover_search_hint"))
            .on_open(|_, cx| Chrome::open_quick_search(cx))
            .size(TriggerSize::Compact)
            .shortcut(shortcut);
        let refresh = Button::new("discover-refresh")
            .custom(style::quiet_button(
                style::white(style::TEXT_STRONG, cx),
                cx,
            ))
            .small()
            .rounded_full()
            .border_1()
            .border_color(style::white(style::BORDER, cx))
            .icon(icons::icon(icons::REFRESH))
            .label(t("refresh"))
            .on_click(cx.listener(|this, _, _, cx| this.ask(true, cx)));
        PageHeader::new(t("discover"))
            .flush()
            .action(refresh)
            .action(search)
    }
}

impl Render for Landing {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("discover-landing");
        components::scroll::smooth("discover-scroll", &self.scroll, window, cx);
        if !self.frozen {
            let sources = SourcesModel::global(cx).read(cx);
            self.caps = sources.is_loaded().then(|| sources.capabilities());
        }
        let landing = match &self.caps {
            Some(caps) => rows::landing(caps, &self.rows, &self.page_rows),
            None => rows::Landing::Waiting,
        };
        let body = match landing {
            rows::Landing::Rows(planned) => v_flex()
                .w_full()
                .gap_10()
                .children(
                    planned
                        .into_iter()
                        .map(|(row, fill)| self.row(row, fill, window, cx)),
                )
                .into_any_element(),
            rows::Landing::Waiting => placeholder_bands(cx),
            rows::Landing::Empty => div()
                .pt_16()
                .child(
                    EmptyState::new(t("catalog_page_empty"))
                        .detail(t("discover_search_hint"))
                        .icon(icons::COMPASS),
                )
                .into_any_element(),
            rows::Landing::Failed(error) => div()
                .pt_16()
                .child(EmptyState::failed(
                    states::catalog_failure(&error),
                    cx.listener(|this, _, _, cx| this.ask(true, cx)),
                ))
                .into_any_element(),
        };
        div()
            .id("discover")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                v_flex()
                    .w_full()
                    .px_8()
                    .pt_8()
                    .pb_10()
                    .gap_8()
                    .child(self.header(cx))
                    .child(body),
            )
    }
}

fn row_key(page: &PageEntry) -> String {
    format!("page-{}", page.id)
}

/// A row's stable name, for its element ids and its fade.
fn row_name(row: Row, pages: &[PageEntry]) -> String {
    match row {
        Row::Mixes => MIXES.to_string(),
        Row::Page(index) => pages.get(index).map(row_key).unwrap_or_default(),
        Row::NeverPlayed => "never-played".to_string(),
        Row::ForgottenFavorites => "forgotten-favorites".to_string(),
    }
}

/// A titled row of pulsing tiles, where a row will be once its read is in.
fn placeholder_row(name: &str, title: SharedString) -> AnyElement {
    let prefix = SharedString::from(format!("discover-pulse-{name}"));
    Carousel::new(
        SharedString::from(format!("{prefix}-row")),
        title,
        PLACEHOLDERS,
        TILE,
        move |slot, _, cx| pulsing(&prefix, slot, cx),
    )
    .gap(GAP)
    .into_any_element()
}

fn pulsing(prefix: &SharedString, slot: usize, cx: &App) -> AnyElement {
    div()
        .child(skeleton::shelf_tile(TILE, cx))
        .with_animation(
            SharedString::from(format!("{prefix}-{slot}")),
            Animation::new(Duration::from_millis(1100))
                .repeat()
                .with_easing(pulsating_between(0.4, 1.)),
            |tile, delta| tile.opacity(delta),
        )
        .into_any_element()
}

/// Albums nobody has played: each tile opens its album.
fn albums_row(title: SharedString, albums: &[AlbumInfo], handle: &ScrollHandle) -> AnyElement {
    let albums: std::rc::Rc<[AlbumInfo]> = albums.into();
    Carousel::new(
        "discover-never-played",
        title,
        albums.len(),
        TILE,
        move |index, _, _| {
            let album = &albums[index];
            components::album_tile(album).size(TILE).into_any_element()
        },
    )
    .gap(GAP)
    .handle(handle.clone())
    .into_any_element()
}

/// Favourites gone quiet: each tile starts a radio of its own, as Home's For you tiles do.
fn tracks_row(title: SharedString, tracks: &[TrackInfo], handle: &ScrollHandle) -> AnyElement {
    let tracks: std::rc::Rc<[TrackInfo]> = tracks.into();
    Carousel::new(
        "discover-forgotten-favorites",
        title,
        tracks.len(),
        TILE,
        move |index, _, _| track_tile(&tracks[index]),
    )
    .gap(GAP)
    .handle(handle.clone())
    .into_any_element()
}

fn track_tile(track: &TrackInfo) -> AnyElement {
    let key = track.key.clone();
    let press = key.clone();
    Tile::new(
        SharedString::from(format!("forgotten-{}", track.uid)),
        track.title.clone(),
    )
    .subtitle(track.artist.clone())
    .artwork(track.artwork.as_ref())
    .size(TILE)
    .placeholder(icons::MUSIC)
    .pictured(Pictured::Track {
        artist: track.artist.clone().into(),
        title: track.title.clone().into(),
    })
    .menu(Some(Target::track(track.key.clone(), track.title.clone())))
    .on_click(move |_, _, cx| actions::start_track_radio(key.clone(), cx))
    .play(PlayState::Idle, move |_, _, cx| {
        actions::start_track_radio(press.clone(), cx)
    })
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_title_resolves_in_kopuz() {
        for key in [
            "discover_your_mixes",
            "discover_never_played",
            "discover_forgotten_favorites",
        ] {
            assert_ne!(i18n::t(key), *key, "{key} is missing from en.ftl");
        }
    }

    #[test]
    fn every_row_has_its_own_name() {
        let pages = [
            PageEntry {
                id: "FEmusic_charts".into(),
                ..PageEntry::default()
            },
            PageEntry {
                id: "FEmusic_new_releases".into(),
                ..PageEntry::default()
            },
        ];
        let names: std::collections::HashSet<String> = [
            Row::Mixes,
            Row::Page(0),
            Row::Page(1),
            Row::NeverPlayed,
            Row::ForgottenFavorites,
        ]
        .into_iter()
        .map(|row| row_name(row, &pages))
        .collect();
        assert_eq!(names.len(), 5);
    }
}

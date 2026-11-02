//! One catalog page opened whole: its header, its chips, every shelf in its own layout, and the rest paged in on scroll.

use std::rc::Rc;
use std::time::Duration;

use api::{
    CatalogChip, CatalogDetail, CatalogDetailRequest, CatalogHeader, CatalogItem, CatalogItemKind,
    CatalogShelf, ShelfLayout,
};
use components::format::{t, text};
use components::{
    Chip, ChipSize, DetailActions, DetailHeader, EmptyState, HeaderLayout, Look, PageHeader,
    motion, skeleton,
};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, Context, Entity,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled, Subscription, Task, Window, div,
    pulsating_between, px,
};
use store::{CatalogCache, Link, SourcesModel};

use crate::shelves::{MoreOf, Shelves, TILE};
use crate::states;

// How close to the end a scroll asks for the next page.
const PAGE_MARGIN: Pixels = px(600.);
/// A page changing to another chip keeps its old shelves up, dimmed, until the new ones land.
const FADED: f32 = 0.55;

/// Which cursor a load is following: the page's own, or one shelf's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Paging {
    Page,
    Shelf(usize),
}

pub struct CatalogPage {
    /// The page the route opened; empty for the source's plain feed, which no page names.
    id: String,
    // Drawn inside Home, which owns the scroll, the edge and the title.
    embedded: bool,
    // Whether the host wears the first shelf's strapline itself, as Home's For you does.
    strapline_to_host: bool,
    /// What is on screen: the page, or the chip picked over it.
    showing: String,
    // The source can keep marking a chip after it was cleared, so the local pick wins.
    picked: Option<Option<String>>,
    detail: Option<CatalogDetail>,
    // Whether the first shelf held is the page's own track list.
    tracks_first: bool,
    shelves: Entity<Shelves>,
    loading: bool,
    paging: Option<Paging>,
    error: Option<SharedString>,
    // Bumped per load; a slow answer for one chip must not land after the user moved to another.
    generation: u64,
    // The shelves' first appearance, kept here so a revisit does not fade them in again.
    reveal: motion::Fade,
    scroll: ScrollHandle,
    _load: Option<Task<()>>,
    _more: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl CatalogPage {
    pub fn new(id: SharedString, _: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(id.to_string(), false, ScrollHandle::new(), cx)
    }

    /// The source's first page, or its plain feed when it lists none, as a section of a page that scrolls it.
    pub fn embedded(
        id: Option<SharedString>,
        scroll: ScrollHandle,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let id = id.map(|id| id.to_string()).unwrap_or_default();
        Self::build(id, true, scroll, cx)
    }

    fn build(id: String, embedded: bool, scroll: ScrollHandle, cx: &mut Context<Self>) -> Self {
        let feed = id.is_empty();
        let shelves = cx.new(|cx| Shelves::new(cx).offer_more(!feed));
        let subscriptions = vec![cx.subscribe(&shelves, |this, _, more: &MoreOf, cx| {
            this.top_up_shelf(more.0, cx)
        })];
        let mut page = Self {
            showing: id.clone(),
            id,
            embedded,
            strapline_to_host: embedded,
            picked: None,
            detail: None,
            tracks_first: false,
            shelves,
            loading: false,
            paging: None,
            error: None,
            generation: 0,
            reveal: motion::Fade::hidden(),
            scroll,
            _load: None,
            _more: None,
            _subscriptions: subscriptions,
        };
        page.load(cx);
        page
    }

    fn is_feed(&self) -> bool {
        self.showing.is_empty()
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.error = None;
        self.paging = None;
        self._more = None;
        // A page seen this session draws at once; a stale one is fetched again behind it.
        let cached = CatalogCache::get(&self.showing, cx);
        let held = cached.is_some();
        if let Some(cached) = cached {
            self.show(cached.detail, cx);
            if cached.fresh {
                self.loading = false;
                self._load = None;
                cx.notify();
                return;
            }
        }
        self.loading = !held;
        cx.notify();
        let showing = self.showing.clone();
        let call = Link::global(cx).call(move |api| async move {
            match showing.is_empty() {
                true => api.catalog(None).await.map(feed_detail),
                false => {
                    api.catalog_detail(CatalogDetailRequest::page(showing))
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
                        CatalogCache::put(&this.showing, detail.clone(), cx);
                        this.show(detail, cx);
                    }
                    // A stale page on screen beats an error over it.
                    Err(error) if this.detail.is_some() => {
                        tracing::warn!(%error, "catalog page refresh failed");
                    }
                    Err(error) => this.error = Some(states::catalog_failure(&error)),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn show(&mut self, detail: CatalogDetail, cx: &mut Context<Self>) {
        let mut held = Vec::with_capacity(detail.shelves.len() + 1);
        self.tracks_first = !detail.tracks.is_empty();
        if self.tracks_first {
            held.push(tracks_shelf(&detail.tracks));
        }
        held.extend(detail.shelves.iter().cloned());
        // Home moves the feed's opening line up to its own first row.
        if self.strapline_to_host
            && let Some(first) = held.first_mut()
        {
            first.strapline = None;
        }
        self.shelves.update(cx, |shelves, cx| shelves.set(held, cx));
        self.detail = Some(detail);
    }

    /// Whether the host draws the first shelf's strapline; off, the feed keeps it, as when Home hides For you.
    pub fn hand_strapline_to_host(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.strapline_to_host == on {
            return;
        }
        self.strapline_to_host = on;
        if let Some(detail) = self.detail.clone() {
            self.show(detail, cx);
        }
        cx.notify();
    }

    /// The chip's page, or back to the page itself when the chip on is pressed again.
    fn pick(&mut self, chip: CatalogChip, cx: &mut Context<Self>) {
        let on = self.chip_on(&chip);
        match on {
            true => {
                self.picked = Some(None);
                self.showing = self.id.clone();
            }
            false => {
                self.picked = Some(Some(chip.id.clone()));
                self.showing = chip.id;
            }
        }
        self.load(cx);
    }

    fn chip_on(&self, chip: &CatalogChip) -> bool {
        match &self.picked {
            Some(on) => on.as_deref() == Some(chip.id.as_str()),
            None => chip.selected,
        }
    }

    /// The cursor scrolling follows: the page's, else its last shelf's, since a long list is one shelf.
    fn scroll_target(&self) -> Option<(Paging, String)> {
        let detail = self.detail.as_ref()?;
        if let Some(token) = detail.continuation.clone() {
            return Some((Paging::Page, token));
        }
        // The plain feed pages only as a whole.
        if self.is_feed() {
            return None;
        }
        let last = detail.shelves.len().checked_sub(1)?;
        let token = detail.shelves[last].continuation.clone()?;
        Some((Paging::Shelf(last), token))
    }

    fn page_on_scroll(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.paging.is_some() {
            return;
        }
        let scrolled = -self.scroll.offset().y;
        if self.scroll.max_offset().y - scrolled > PAGE_MARGIN {
            return;
        }
        if let Some((paging, token)) = self.scroll_target() {
            self.load_more(paging, token, cx);
        }
    }

    /// "Show more" under a held shelf; the first held shelf may be the page's tracks, which never page.
    fn top_up_shelf(&mut self, held: usize, cx: &mut Context<Self>) {
        let Some(index) = held.checked_sub(usize::from(self.tracks_first)) else {
            return;
        };
        let token = self
            .detail
            .as_ref()
            .and_then(|detail| detail.shelves.get(index))
            .and_then(|shelf| shelf.continuation.clone());
        if let Some(token) = token {
            self.load_more(Paging::Shelf(index), token, cx);
        }
    }

    fn load_more(&mut self, paging: Paging, token: String, cx: &mut Context<Self>) {
        if self.paging.is_some() {
            return;
        }
        self.paging = Some(paging);
        let held = self.held_index(paging);
        self.shelves
            .update(cx, |shelves, cx| shelves.set_topping_up(held, cx));
        let generation = self.generation;
        let feed = self.is_feed();
        let request = CatalogDetailRequest {
            continuation: Some(token.clone()),
            ..CatalogDetailRequest::page(self.showing.clone())
        };
        let call = Link::global(cx).call(move |api| async move {
            match feed {
                true => api.catalog(Some(token)).await.map(feed_detail),
                false => api.catalog_detail(request).await,
            }
        });
        cx.notify();
        self._more = Some(cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.paging = None;
                this.shelves
                    .update(cx, |shelves, cx| shelves.set_topping_up(None, cx));
                match result {
                    Ok(answer) => {
                        this.extend(paging, answer, cx);
                        if let Some(detail) = this.detail.clone() {
                            CatalogCache::update(&this.showing, detail, cx);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "catalog page continuation failed");
                        // Stop asking: a failed cursor fails again on every scroll.
                        this.forget_cursor(paging);
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn held_index(&self, paging: Paging) -> Option<usize> {
        match paging {
            Paging::Page => None,
            Paging::Shelf(index) => Some(index + usize::from(self.tracks_first)),
        }
    }

    fn forget_cursor(&mut self, paging: Paging) {
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        match paging {
            Paging::Page => detail.continuation = None,
            Paging::Shelf(index) => {
                if let Some(shelf) = detail.shelves.get_mut(index) {
                    shelf.continuation = None;
                }
            }
        }
    }

    fn extend(&mut self, paging: Paging, answer: CatalogDetail, cx: &mut Context<Self>) {
        let tracks_first = self.tracks_first;
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        match paging {
            Paging::Page => {
                detail.continuation = answer.continuation;
                let tracks = answer.tracks;
                if !tracks.is_empty() {
                    detail.tracks.extend(tracks.iter().cloned());
                    let items = tracks_shelf(&tracks).items;
                    match tracks_first {
                        true => self
                            .shelves
                            .update(cx, |shelves, cx| shelves.top_up(0, items, None, cx)),
                        false => {
                            let shelf = tracks_shelf(&tracks);
                            detail.shelves.push(shelf.clone());
                            self.shelves
                                .update(cx, |shelves, cx| shelves.extend(vec![shelf], cx));
                        }
                    }
                }
                detail.shelves.extend(answer.shelves.iter().cloned());
                self.shelves
                    .update(cx, |shelves, cx| shelves.extend(answer.shelves, cx));
            }
            Paging::Shelf(index) => {
                let Some(shelf) = detail.shelves.get_mut(index) else {
                    return;
                };
                let more = answer.shelves.into_iter().next();
                let (items, continuation) = match more {
                    Some(more) => (more.items, more.continuation),
                    None => (Vec::new(), None),
                };
                shelf.items.extend(items.iter().cloned());
                shelf.continuation = continuation.clone();
                let held = index + usize::from(tracks_first);
                self.shelves.update(cx, |shelves, cx| {
                    shelves.top_up(held, items, continuation, cx)
                });
            }
        }
    }

    /// The page's own title, or the label the source declared it under while it has none.
    fn title(&self, cx: &App) -> SharedString {
        if let Some(detail) = self
            .detail
            .as_ref()
            .filter(|detail| !detail.title.is_empty())
        {
            return detail.title.clone().into();
        }
        let sources = SourcesModel::global(cx);
        let caps = sources.read(cx).capabilities();
        caps.pages
            .iter()
            .find(|page| page.id == self.id)
            .map(|page| text(&page.label))
            .unwrap_or_default()
    }

    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let title = self.title(cx);
        let Some(detail) = self.detail.as_ref() else {
            return PageHeader::new(title).into_any_element();
        };
        let layout = match detail.header {
            CatalogHeader::None | CatalogHeader::Title => {
                return PageHeader::new(title).into_any_element();
            }
            CatalogHeader::Detail => HeaderLayout::Row,
            CatalogHeader::Artist => HeaderLayout::Banner,
        };
        let play = detail.playback_id.clone().map(|id| {
            DetailActions::new("catalog-page", Rc::<[String]>::from([]))
                .look(Look::Compact)
                .play_with(cx.listener(move |this, _, _, cx| {
                    let id = id.clone();
                    this.shelves.update(cx, |shelves, cx| {
                        shelves.play(CatalogItemKind::Playlist, id, cx)
                    });
                }))
        });
        DetailHeader::new(title)
            .layout(layout)
            .artwork(detail.artwork.as_ref())
            .when_some(detail.subtitle.clone(), |header, subtitle| {
                header.meta(subtitle)
            })
            .description(detail.description.clone())
            .when_some(play, |header, play| header.action(play))
            .into_any_element()
    }

    fn chips(&self, page: &Entity<Self>) -> Option<AnyElement> {
        let chips = &self.detail.as_ref()?.chips;
        if chips.is_empty() {
            return None;
        }
        let row = chips.iter().enumerate().map(|(at, chip)| {
            let (picked, page) = (chip.clone(), page.clone());
            Chip::new(("catalog-chip", at))
                .label(chip.label.clone())
                .size(ChipSize::Medium)
                .selected(self.chip_on(chip))
                .on_click(move |_, _, cx| page.update(cx, |this, cx| this.pick(picked.clone(), cx)))
        });
        Some(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .children(row)
                .into_any_element(),
        )
    }
}

impl Render for CatalogPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("catalog-page");
        // Embedded, the host draws the chips through `CatalogChips`.
        let chips = match self.embedded {
            true => None,
            false => self.chips(&cx.entity()),
        };
        let held = self.detail.as_ref();
        let empty =
            held.is_some_and(|detail| detail.tracks.is_empty() && detail.shelves.is_empty());
        let first_load = self.loading && held.is_none();
        let switching = self.loading && held.is_some();
        let paging_page = self.paging == Some(Paging::Page);
        let reveal = self
            .reveal
            .sample(held.is_some(), motion::MEDIUM, window, cx);
        let content = v_flex()
            .w_full()
            .gap_8()
            .children(chips)
            .when_some(self.error.clone(), |page, message| {
                page.child(EmptyState::failed(
                    message,
                    cx.listener(|this, _, _, cx| this.load(cx)),
                ))
            })
            .when(first_load, |page| page.child(placeholder_bands(cx)))
            .when(empty && !self.loading, |page| {
                page.child(EmptyState::new(t("catalog_page_empty")).icon(components::icons::FOLDER))
            })
            .when(held.is_some(), |page| {
                page.child(
                    div()
                        .w_full()
                        .opacity(if switching { FADED } else { reveal })
                        .child(self.shelves.clone()),
                )
            })
            .when(paging_page, |page| {
                page.child(
                    components::Loading::new()
                        .label(t("discover_more_loading"))
                        .compact(),
                )
            });
        if self.embedded {
            // The host scrolls; a wheel over the feed is what asks it for more.
            return div()
                .id("catalog-home")
                .w_full()
                .on_scroll_wheel(cx.listener(|this, _, _, cx| this.page_on_scroll(cx)))
                .child(content)
                .into_any_element();
        }
        components::scroll::smooth("catalog-page-scroll", &self.scroll, window, cx);
        let header = self.header(cx);
        div()
            .id("catalog-page")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .on_scroll_wheel(cx.listener(|this, _, _, cx| this.page_on_scroll(cx)))
            .child(
                v_flex()
                    .w_full()
                    .px_8()
                    .pt_8()
                    .pb_10()
                    .gap_8()
                    .child(header)
                    .child(content),
            )
            .into_any_element()
    }
}

/// An embedded page's chips, for a host that draws them apart from the feed they switch.
pub struct CatalogChips {
    page: Entity<CatalogPage>,
    _observe: Subscription,
}

impl CatalogChips {
    pub fn new(page: Entity<CatalogPage>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&page, |_, _, cx| cx.notify());
        Self {
            page,
            _observe: observe,
        }
    }
}

impl Render for CatalogChips {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Its own space below, so a source without chips leaves no gap over the row it heads.
        let row = self.page.read(cx).chips(&self.page);
        div()
            .w_full()
            .when_some(row, |this, row| this.pb_6().child(row))
    }
}

/// The plain feed's answer, in the shape a page holds.
fn feed_detail(page: api::CatalogPage) -> CatalogDetail {
    CatalogDetail {
        shelves: page.shelves,
        continuation: page.continuation,
        ..CatalogDetail::default()
    }
}

/// The page's own songs, as the list shelf that leads it.
fn tracks_shelf(tracks: &[api::TrackInfo]) -> CatalogShelf {
    CatalogShelf {
        items: tracks
            .iter()
            .map(|track| CatalogItem {
                kind: CatalogItemKind::Track,
                id: track.key.clone(),
                title: track.title.clone(),
                track: Some(track.clone()),
                ..CatalogItem::default()
            })
            .collect(),
        layout: ShelfLayout::List,
        ..CatalogShelf::default()
    }
}

/// Two bands of pulsing tiles where the first shelves will be.
pub(crate) fn placeholder_bands(cx: &App) -> AnyElement {
    v_flex()
        .w_full()
        .gap_10()
        .children((0..2).map(|band| {
            div()
                .child(skeleton::band(6, TILE, false, cx))
                .with_animation(
                    SharedString::from(format!("catalog-page-pulse-{band}")),
                    Animation::new(Duration::from_millis(1100))
                        .repeat()
                        .with_easing(pulsating_between(0.4, 1.)),
                    |band, delta| band.opacity(delta),
                )
        }))
        .into_any_element()
}

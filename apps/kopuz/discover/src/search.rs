//! Search on a source that files its results by kind: the filter chips, the source's own shelves, and the rest paged in on scroll.

use api::{CatalogShelf, SearchFilter, SearchRequest, SearchResults, ShelfLayout};
use components::format::{t, t_with, text};
use components::{Chip, ChipSize, EmptyState, icons, style};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Task, Window, div, point, px,
};
use store::Link;

use crate::page::placeholder_bands;
use crate::shelves::{Shelves, ShowFilter};
use crate::states;

// How close to the end a scroll asks for the next page.
const PAGE_MARGIN: Pixels = px(600.);
/// Shelves answering the previous query or filter stay up, dimmed, until the new ones land.
const FADED: f32 = 0.55;

/// Which ask an answer belongs to: a newer ask, or freezing the page, leaves every earlier answer unclaimed.
#[derive(Debug, Default)]
struct Asks {
    current: u64,
    frozen: bool,
}

impl Asks {
    /// A new ask, or none once frozen: a frozen page draws what it holds and asks nothing more.
    fn begin(&mut self) -> Option<u64> {
        if self.frozen {
            return None;
        }
        self.current += 1;
        Some(self.current)
    }

    /// The ask an extra page continues, which a fresh ask supersedes like any other answer.
    fn ongoing(&self) -> Option<u64> {
        (!self.frozen).then_some(self.current)
    }

    fn answers(&self, ask: u64) -> bool {
        !self.frozen && ask == self.current
    }

    fn freeze(&mut self) {
        self.frozen = true;
        self.current += 1;
    }
}

pub(crate) struct CatalogSearch {
    query: String,
    filters: Vec<SearchFilter>,
    filter: String,
    shelves: Entity<Shelves>,
    // How many shelves are held, so a continuation knows which one it extends.
    held: Option<usize>,
    continuation: Option<String>,
    correction: Option<String>,
    loading: bool,
    paging: bool,
    error: Option<SharedString>,
    // An answer for a query or filter since left behind, or one landing after a freeze, is dropped.
    asks: Asks,
    scroll: ScrollHandle,
    _load: Option<Task<()>>,
    _more: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl CatalogSearch {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let shelves = cx.new(|cx| Shelves::new(cx).searching(true));
        let subscriptions = vec![cx.subscribe(&shelves, |this, _, show: &ShowFilter, cx| {
            this.pick(show.0.clone(), cx)
        })];
        Self {
            query: String::new(),
            filters: Vec::new(),
            filter: String::new(),
            shelves,
            held: None,
            continuation: None,
            correction: None,
            loading: false,
            paging: false,
            error: None,
            asks: Asks::default(),
            scroll: ScrollHandle::new(),
            _load: None,
            _more: None,
            _subscriptions: subscriptions,
        }
    }

    /// Search `query` under the source's `filters`; a pick that is still offered stays picked.
    pub(crate) fn set(
        &mut self,
        query: String,
        filters: Vec<SearchFilter>,
        cx: &mut Context<Self>,
    ) {
        if query == self.query && filters == self.filters {
            return;
        }
        if !filters.iter().any(|filter| filter.id == self.filter) {
            self.filter = filters
                .first()
                .map(|filter| filter.id.clone())
                .unwrap_or_default();
        }
        self.query = query;
        self.filters = filters;
        self.load(cx);
    }

    /// Another source answers differently, so its first ask is a new one whatever the box holds.
    pub(crate) fn forget(&mut self, cx: &mut Context<Self>) {
        self.query.clear();
        self.filters.clear();
        self.filter.clear();
        self.load(cx);
    }

    fn pick(&mut self, filter: String, cx: &mut Context<Self>) {
        if filter == self.filter || !self.filters.iter().any(|offered| offered.id == filter) {
            return;
        }
        self.filter = filter;
        self.load(cx);
    }

    /// The first filter is the source's "all", which lays shelves out as it likes.
    fn is_filtered(&self) -> bool {
        self.filters
            .first()
            .is_some_and(|all| all.id != self.filter)
    }

    /// Stop asking and drop what is in flight, so a source switch's frozen page takes no late answer.
    pub(crate) fn freeze(&mut self, cx: &mut Context<Self>) {
        self.asks.freeze();
        self._load = None;
        self._more = None;
        self.loading = false;
        self.paging = false;
        cx.notify();
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(generation) = self.asks.begin() else {
            return;
        };
        self.continuation = None;
        self.correction = None;
        self.error = None;
        self.paging = false;
        self._more = None;
        self.scroll.set_offset(point(px(0.), px(0.)));
        if self.query.is_empty() || self.filter.is_empty() {
            self.loading = false;
            self.held = None;
            self._load = None;
            self.shelves
                .update(cx, |shelves, cx| shelves.set(Vec::new(), cx));
            cx.notify();
            return;
        }
        self.loading = true;
        cx.notify();
        let request = SearchRequest::filtered(self.query.clone(), self.filter.clone());
        let call = Link::global(cx).call(move |api| async move { api.search(request).await });
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                if !this.asks.answers(generation) {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(results) => this.show(results, cx),
                    Err(error) => {
                        this.held = None;
                        this.error = Some(states::catalog_failure(&error));
                        this.shelves
                            .update(cx, |shelves, cx| shelves.set(Vec::new(), cx));
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn show(&mut self, results: SearchResults, cx: &mut Context<Self>) {
        let filtered = self.is_filtered();
        let shelves: Vec<CatalogShelf> = results
            .shelves
            .into_iter()
            .filter(|shelf| !shelf.items.is_empty())
            .map(
                |shelf| match filtered && shelf.layout == ShelfLayout::Carousel {
                    // One filter is one shelf, so it has the room to wrap.
                    true => CatalogShelf {
                        layout: ShelfLayout::Grid,
                        ..shelf
                    },
                    false => shelf,
                },
            )
            .collect();
        self.held = Some(shelves.len());
        self.continuation = results.continuation;
        self.correction = results.correction;
        self.shelves.update(cx, |held, cx| held.set(shelves, cx));
    }

    fn page_on_scroll(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.paging {
            return;
        }
        let scrolled = -self.scroll.offset().y;
        if self.scroll.max_offset().y - scrolled > PAGE_MARGIN {
            return;
        }
        if let Some(token) = self.continuation.clone() {
            self.load_more(token, cx);
        }
    }

    /// A continuation answers with one shelf, which continues the last one held.
    fn load_more(&mut self, token: String, cx: &mut Context<Self>) {
        let Some(generation) = self.asks.ongoing() else {
            return;
        };
        self.paging = true;
        let request = SearchRequest {
            query: self.query.clone(),
            filter: Some(self.filter.clone()),
            continuation: Some(token),
        };
        let call = Link::global(cx).call(move |api| async move { api.search(request).await });
        cx.notify();
        self._more = Some(cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                if !this.asks.answers(generation) {
                    return;
                }
                this.paging = false;
                match result {
                    Ok(results) => {
                        this.continuation = results.continuation;
                        if let Some(more) = results.shelves.into_iter().next() {
                            this.extend(more, cx);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "search continuation failed");
                        // Stop asking: a failed cursor fails again on every scroll.
                        this.continuation = None;
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn extend(&mut self, more: CatalogShelf, cx: &mut Context<Self>) {
        match self.held {
            Some(count) if count > 0 => self.shelves.update(cx, |shelves, cx| {
                shelves.top_up(count - 1, more.items, None, cx)
            }),
            _ => {
                self.held = Some(1);
                self.shelves
                    .update(cx, |shelves, cx| shelves.extend(vec![more], cx));
            }
        }
    }

    fn chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let page = cx.entity();
        let row = self.filters.iter().map(|filter| {
            let (id, page) = (filter.id.clone(), page.clone());
            Chip::new(SharedString::from(format!("search-filter-{}", filter.id)))
                .label(text(&filter.label))
                .size(ChipSize::Medium)
                .selected(filter.id == self.filter)
                .on_click(move |_, _, cx| page.update(cx, |this, cx| this.pick(id.clone(), cx)))
        });
        div().flex().flex_wrap().gap_2().children(row)
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(message) = self.error.clone() {
            return EmptyState::failed(message, cx.listener(|this, _, _, cx| this.load(cx)))
                .into_any_element();
        }
        match self.held {
            None if self.loading => placeholder_bands(cx),
            Some(0) if !self.loading => EmptyState::new(t_with(
                "no_results_found",
                &[("query", self.query.clone().into())],
            ))
            .detail(t("discover_no_results_hint"))
            .icon(icons::SEARCH)
            .into_any_element(),
            _ => div()
                .w_full()
                .when(self.loading, |shelves| shelves.opacity(FADED))
                .child(self.shelves.clone())
                .into_any_element(),
        }
    }
}

impl Render for CatalogSearch {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("catalog-search");
        components::scroll::smooth("catalog-search-scroll", &self.scroll, window, cx);
        let correction = self.correction.clone().map(|corrected| {
            div()
                .text_sm()
                .text_color(style::white(style::TEXT_MUTED, cx))
                .child(t_with(
                    "search_showing_results_for",
                    &[("query", corrected.into())],
                ))
        });
        let body = self.body(cx);
        v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap_6()
            .child(self.chips(cx))
            .child(
                div()
                    .id("catalog-search")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_scroll_wheel(cx.listener(|this, _, _, cx| this.page_on_scroll(cx)))
                    .child(
                        v_flex()
                            .w_full()
                            .pb_10()
                            .gap_8()
                            .children(correction)
                            .child(body)
                            .when(self.paging, |page| {
                                page.child(
                                    components::Loading::new()
                                        .label(t("discover_more_loading"))
                                        .compact(),
                                )
                            }),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::Asks;

    #[test]
    fn a_freeze_drops_every_answer_in_flight_and_asks_nothing_more() {
        let mut asks = Asks::default();
        let search = asks.begin().expect("a live page asks");
        let more = asks.ongoing().expect("a live page pages");
        assert!(asks.answers(search) && asks.answers(more));
        asks.freeze();
        assert!(
            !asks.answers(search),
            "a search answer landed on a frozen page"
        );
        assert!(
            !asks.answers(more),
            "a page of more landed on a frozen page"
        );
        assert_eq!(asks.begin(), None);
        assert_eq!(asks.ongoing(), None);
    }

    #[test]
    fn a_new_ask_drops_the_answers_to_the_one_before() {
        let mut asks = Asks::default();
        let first = asks.begin().expect("a live page asks");
        let second = asks.begin().expect("a live page asks");
        assert!(!asks.answers(first));
        assert!(asks.answers(second));
    }

    #[test]
    fn every_search_string_resolves_in_kopuz() {
        for key in [
            "search_showing_results_for",
            "no_results_found",
            "discover_no_results_hint",
            "discover_more_loading",
        ] {
            assert_ne!(i18n::t(key), *key, "{key} is missing from en.ftl");
        }
    }
}

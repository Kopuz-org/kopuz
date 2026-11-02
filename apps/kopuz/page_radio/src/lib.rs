//! Internet radio: the stations the daemon holds pinned, and the public directory it searches.

use std::collections::HashSet;
use std::time::Duration;

use api::{QueueContext, RadioStationInfo, Table};
use components::Loading;
use components::format::t;
use components::{
    EmptyState, Heading, PageHeader, SearchField, SectionHeading, StationRow, StationVariant,
    icons, style,
};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, ListAlignment, ListOffset,
    ListState, ParentElement as _, Render, SharedString, Styled, Subscription, Task, Window, div,
    list, px, rems,
};
use link::Link;
use store::{Notices, Query, actions};

// Typing settles before the directory is searched; the Dioxus page used the same delay.
const DEBOUNCE: Duration = Duration::from_millis(300);
const RESULTS: u32 = 60;

// `max-w-sm w-full`: the directory search box.
fn search_width() -> gpui_kit::Rems {
    rems(24.)
}

/// Registry text may be a translation key, so the filter matches what is on screen.
fn matches(station: &RadioStationInfo, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    t(&station.name).to_lowercase().contains(query)
        || t(&station.description).to_lowercase().contains(query)
        || station
            .streams
            .iter()
            .any(|stream| t(&stream.name).to_lowercase().contains(query))
        || station
            .tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(query))
}

fn play(station_id: String, stream_id: String, cx: &mut gpui_kit::App) {
    actions::play_context(
        QueueContext::Radio {
            station_id,
            stream_id,
        },
        Some(0),
        cx,
    );
}

pub struct Radio {
    filter: Entity<InputState>,
    query: String,
    stations: Entity<Query<Vec<RadioStationInfo>>>,
    directory: Entity<Query<Vec<RadioStationInfo>>>,
    pinned: HashSet<String>,
    list: ListState,
    keys: Vec<SharedString>,
    lines: Vec<Line>,
    _debounce: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Radio {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter =
            cx.new(|cx| InputState::new(window, cx).placeholder(t("radio_search_stations")));
        let stations = cx.new(|cx| {
            Query::new(
                &[Table::Stations],
                |api| async move { api.radio_stations().await },
                cx,
            )
        });
        let directory = cx.new(|cx| {
            Query::new(
                &[],
                |api| async move { api.search_radio(String::new(), RESULTS).await },
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe(&filter, |this, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = state.read(cx).value().to_string();
                    this.debounce(value, cx);
                }
            }),
            cx.observe(&stations, |this, _, cx| this.adopt_pins(cx)),
            cx.observe(&directory, |this, _, cx| this.relist(cx)),
        ];
        Self {
            filter,
            query: String::new(),
            stations,
            directory,
            pinned: HashSet::new(),
            list: ListState::new(0, ListAlignment::Top, px(400.)),
            keys: Vec::new(),
            lines: Vec::new(),
            _debounce: None,
            _subscriptions: subscriptions,
        }
    }

    /// A newer keystroke replaces the pending task, which cancels it.
    fn debounce(&mut self, value: String, cx: &mut Context<Self>) {
        self._debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            this.update(cx, |this, cx| this.apply(value, cx)).ok();
        }));
    }

    fn apply(&mut self, value: String, cx: &mut Context<Self>) {
        if self.query == value {
            return;
        }
        self.query = value;
        let search = self.query.trim().to_string();
        self.directory.update(cx, |query, cx| {
            query.set_fetch(
                move |api| {
                    let search = search.clone();
                    async move { api.search_radio(search, RESULTS).await }
                },
                cx,
            )
        });
        self.relist(cx);
    }

    fn adopt_pins(&mut self, cx: &mut Context<Self>) {
        if let Some(stations) = self.stations.read(cx).value() {
            self.pinned = stations
                .iter()
                .filter(|station| station.pinned)
                .map(|station| station.id.clone())
                .collect();
        }
        self.relist(cx);
    }

    /// Move the local set first: the answer is a round trip away and a check mark that waits reads as a dead button.
    fn toggle_pin(&mut self, id: String, cx: &mut Context<Self>) {
        let pinned = self.pinned.contains(&id);
        if pinned {
            self.pinned.remove(&id);
        } else {
            self.pinned.insert(id.clone());
        }
        self.relist(cx);
        let call = Link::global(cx).call({
            let id = id.clone();
            move |api| async move { api.pin_radio_station(id, !pinned).await }
        });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| match result {
                // The daemon announces the new list on the Stations table, which refetches it.
                Ok(()) => {}
                Err(error) => {
                    tracing::warn!(%error, "pinning a station failed");
                    if pinned {
                        this.pinned.insert(id);
                    } else {
                        this.pinned.remove(&id);
                    }
                    Notices::failed(&error, cx);
                    this.relist(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// What a station row does with a stream it is asked to play.
    fn player(station: &RadioStationInfo) -> impl Fn(&str, &mut Window, &mut App) + use<> {
        let id = station.id.clone();
        move |stream, _, cx| play(id.clone(), stream.to_string(), cx)
    }

    /// `text-sm font-bold uppercase tracking-wider text-slate-400`, less the two GPUI cannot draw.
    fn heading(heading: SectionHeading) -> impl IntoElement {
        div()
            .w_full()
            .pt_4()
            .pb_3()
            .child(heading.heading(Heading::Group))
    }

    /// What the list shows, in order: the pinned stations, then the directory or its state.
    fn lines(&self, cx: &App) -> (Vec<SharedString>, Vec<Line>) {
        let query = self.query.trim().to_lowercase();
        let mut keys = Vec::new();
        let mut lines = Vec::new();
        let pinned: Vec<RadioStationInfo> = self
            .stations
            .read(cx)
            .value()
            .map(|stations| {
                stations
                    .iter()
                    .filter(|station| self.pinned.contains(&station.id) && matches(station, &query))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if !pinned.is_empty() {
            keys.push(SharedString::new_static("selected"));
            lines.push(Line::SelectedHeading);
            let last = pinned.len() - 1;
            for (at, station) in pinned.into_iter().enumerate() {
                keys.push(SharedString::from(format!("pinned-{}", station.id)));
                lines.push(Line::Pinned(station, at == last));
            }
        }
        keys.push(SharedString::new_static("directory"));
        lines.push(Line::DirectoryHeading);
        let directory = self.directory.read(cx);
        let status = if directory.is_loading() || !directory.is_settled() {
            Some(Status::Loading)
        } else if let Some(error) = directory.error() {
            Some(Status::Failed(store::human(error)))
        } else if directory.value().is_none_or(|stations| stations.is_empty()) {
            Some(Status::Empty)
        } else {
            None
        };
        match status {
            Some(status) => {
                keys.push(SharedString::new_static("status"));
                lines.push(Line::Status(status));
            }
            None => {
                for station in directory.value().cloned().unwrap_or_default() {
                    keys.push(SharedString::from(format!("station-{}", station.id)));
                    lines.push(Line::Station(station));
                }
            }
        }
        (keys, lines)
    }

    /// Rebuild the lines from what changed, not per frame: render only reads them.
    fn relist(&mut self, cx: &mut Context<Self>) {
        let (keys, lines) = self.lines(cx);
        self.adopt_lines(keys, lines);
        cx.notify();
    }

    /// Point the list at new lines, keeping whichever line was at the top where it was.
    fn adopt_lines(&mut self, keys: Vec<SharedString>, lines: Vec<Line>) {
        self.lines = lines;
        if keys == self.keys {
            return;
        }
        let top = self.list.logical_scroll_top();
        let anchor = self.keys.get(top.item_ix).cloned();
        self.list.reset(keys.len());
        if let Some(item_ix) = anchor.and_then(|anchor| keys.iter().position(|key| *key == anchor))
        {
            self.list.scroll_to(ListOffset {
                item_ix,
                offset_in_item: top.offset_in_item,
            });
        }
        self.keys = keys;
    }

    fn line(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(line) = self.lines.get(index).cloned() else {
            return div().into_any_element();
        };
        match line {
            Line::SelectedHeading => {
                Self::heading(SectionHeading::new(t("radio_selected"))).into_any_element()
            }
            Line::Pinned(station, last) => {
                let list = self.list.clone();
                let row = StationRow::new(&station)
                    .on_play(Self::player(&station))
                    .on_resize(move |_, _| list.remeasure());
                div()
                    .w_full()
                    .map(|slot| match last {
                        true => slot.pb_8(),
                        false => slot.pb_3(),
                    })
                    .child(row)
                    .into_any_element()
            }
            Line::DirectoryHeading => {
                let searching = !self.query.trim().is_empty();
                let title = match searching {
                    true => t("radio_search_results"),
                    false => t("radio_top_stations"),
                };
                Self::heading(
                    SectionHeading::new(title).trailing(
                        div()
                            .text_size(style::text_10())
                            .text_color(style::white(0.25, cx))
                            .child(t("radio_powered_by")),
                    ),
                )
                .into_any_element()
            }
            Line::Status(Status::Loading) => Loading::new()
                .label(t("radio_loading_stations"))
                .into_any_element(),
            Line::Status(Status::Failed(error)) => EmptyState::failed(
                t("radio_search_failed"),
                cx.listener(|this, _, _, cx| {
                    this.directory.update(cx, |query, cx| query.refresh(cx));
                }),
            )
            .detail(error)
            .into_any_element(),
            Line::Status(Status::Empty) => EmptyState::new(t("radio_no_stations_match"))
                .icon(icons::RADIO)
                .into_any_element(),
            Line::Station(station) => {
                let this = cx.entity().downgrade();
                let id = station.id.clone();
                let row = StationRow::new(&station)
                    .variant(StationVariant::Directory)
                    .pinned(self.pinned.contains(&station.id))
                    .on_play(Self::player(&station))
                    .on_pin(move |_, cx| {
                        this.update(cx, |this, cx| this.toggle_pin(id.clone(), cx))
                            .ok();
                    });
                div().w_full().pb_3().child(row).into_any_element()
            }
        }
    }
}

/// What the directory says in place of stations.
#[derive(Clone)]
enum Status {
    Loading,
    Failed(SharedString),
    Empty,
}

/// One line of the page's list; only the ones on screen are ever built.
#[derive(Clone)]
enum Line {
    SelectedHeading,
    // The last pinned row carries the gap before the directory.
    Pinned(RadioStationInfo, bool),
    DirectoryHeading,
    Status(Status),
    Station(RadioStationInfo),
}

impl Render for Radio {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        components::page()
            .child(
                div().w_full().mb_8().child(
                    PageHeader::new(t("radio"))
                        .icon(icons::RADIO)
                        .subtitle(t("radio_subtitle"))
                        .flush()
                        .action(
                            div()
                                .w(search_width())
                                .flex_shrink_0()
                                .child(SearchField::new(&self.filter).cleanable(true)),
                        ),
                ),
            )
            .child(
                // Virtualized: a whole directory rebuilt on every scrolled frame is what made this page lag.
                list(
                    self.list.clone(),
                    cx.processor(|this, index: usize, _, cx| this.line(index, cx)),
                )
                .flex_1()
                .min_h_0()
                .w_full(),
            )
    }
}

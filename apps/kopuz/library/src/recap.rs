//! The Activity page's recap: listening over a period the listener picks, ending now.

use api::{ListeningRecap, RecapEntry, RecapPeak, RecapPeriod, RecapUnit, Table};
use components::format::{t, t_with};
use components::{EmptyState, Loading, RankedList, Segment, Segmented, TileRow, icons};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div,
};
use store::{Query, Route};

use crate::bar::stat_card;

// A counted listen announces itself as a recents change.
const TABLES: &[Table] = &[Table::Tracks, Table::Recents];

pub(crate) struct Recap {
    period: RecapPeriod,
    recap: Entity<Query<ListeningRecap>>,
    scroll: ScrollHandle,
    _observe: Subscription,
}

fn period_label(period: RecapPeriod) -> SharedString {
    t(match period {
        RecapPeriod::Day => "recap_period_day",
        RecapPeriod::Week => "recap_period_week",
        RecapPeriod::Month => "recap_period_month",
        RecapPeriod::Year => "recap_period_year",
        RecapPeriod::AllTime => "recap_period_all_time",
    })
}

fn period_key(period: RecapPeriod) -> &'static str {
    match period {
        RecapPeriod::Day => "day",
        RecapPeriod::Week => "week",
        RecapPeriod::Month => "month",
        RecapPeriod::Year => "year",
        RecapPeriod::AllTime => "all-time",
    }
}

/// The busiest stretch's label and value, in the unit its period counts in.
fn peak_text(peak: &RecapPeak) -> (SharedString, SharedString) {
    let month = || t_with("recap_month", &[("month", peak.month.into())]).to_string();
    match peak.unit {
        RecapUnit::Hour => (
            t("recap_busiest_hour"),
            t_with(
                "recap_peak_hour",
                &[("hour", format!("{:02}", peak.hour).into())],
            ),
        ),
        RecapUnit::Day => (
            t("recap_busiest_day"),
            t_with(
                "recap_peak_day",
                &[("month", month().into()), ("day", peak.day.into())],
            ),
        ),
        RecapUnit::Month => (
            t("recap_busiest_month"),
            t_with(
                "recap_peak_month",
                &[
                    ("month", month().into()),
                    ("year", peak.year.to_string().into()),
                ],
            ),
        ),
    }
}

impl Recap {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let period = RecapPeriod::default();
        let recap = cx.new(|cx| {
            Query::new(
                TABLES,
                move |api| async move { api.listening_recap(period).await },
                cx,
            )
        });
        let observe = cx.observe(&recap, |_, _, cx| cx.notify());
        Self {
            period,
            recap,
            scroll: ScrollHandle::new(),
            _observe: observe,
        }
    }

    fn pick(&mut self, period: RecapPeriod, cx: &mut Context<Self>) {
        if self.period == period {
            return;
        }
        self.period = period;
        self.recap.update(cx, |query, cx| {
            query.set_fetch(
                move |api| async move { api.listening_recap(period).await },
                cx,
            )
        });
        cx.notify();
    }

    fn period_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let page = cx.entity().downgrade();
        div()
            .flex()
            .self_start()
            .child(
                Segmented::new("recap-period")
                    .pill(true)
                    .segments(RecapPeriod::ALL.iter().map(|period| {
                        Segment::new(period_key(*period)).label(period_label(*period))
                    }))
                    .selected(
                        RecapPeriod::ALL
                            .iter()
                            .position(|period| *period == self.period),
                    )
                    .on_select(move |index, _, cx| {
                        let Some(period) = RecapPeriod::ALL.get(*index).copied() else {
                            return;
                        };
                        page.update(cx, |this, cx| this.pick(period, cx)).ok();
                    }),
            )
            .into_any_element()
    }

    fn summary(&self, recap: &ListeningRecap, cx: &mut Context<Self>) -> AnyElement {
        let minutes = (recap.listened_ms / 60_000).to_string();
        let (peak_label, peak_value) = recap.peak.as_ref().map(peak_text).unwrap_or_default();
        let streak = t_with(
            "recap_streak_days",
            &[("count", recap.longest_streak_days.into())],
        );
        let highlights = [
            (
                t("recap_minutes_listened"),
                icons::CLOCK,
                SharedString::from(minutes),
            ),
            (t("plays"), icons::PLAY, recap.plays.to_string().into()),
            (peak_label, icons::HISTORY, peak_value),
            (t("recap_streak"), icons::FLAME, streak),
        ];
        let counts = [
            (t("tracks"), icons::MUSIC, recap.tracks),
            (t("artists"), icons::MIC, recap.artists),
            (t("albums"), icons::DISC, recap.albums),
        ];
        v_flex()
            .w_full()
            .gap_3()
            .child(
                h_flex().w_full().gap_3().children(
                    highlights
                        .into_iter()
                        .map(|(label, icon, value)| stat_card(label, icon, value, cx)),
                ),
            )
            .child(
                h_flex().w_full().gap_3().children(
                    counts
                        .into_iter()
                        .map(|(label, icon, value)| stat_card(label, icon, value.to_string(), cx)),
                ),
            )
            .into_any_element()
    }

    fn loaded(&self, recap: &ListeningRecap, cx: &mut Context<Self>) -> AnyElement {
        if recap.plays == 0 {
            return EmptyState::new(t("recap_empty"))
                .icon(icons::ACTIVITY)
                .flex()
                .into_any_element();
        }
        v_flex()
            .w_full()
            .gap_8()
            .child(self.summary(recap, cx))
            .child(
                h_flex()
                    .w_full()
                    .gap_8()
                    .items_start()
                    .child(ranking("recap_top_tracks", Kind::Track, &recap.top_tracks))
                    .child(ranking(
                        "recap_top_artists",
                        Kind::Artist,
                        &recap.top_artists,
                    ))
                    .child(ranking("recap_top_albums", Kind::Album, &recap.top_albums)),
            )
            .into_any_element()
    }
}

/// One ranking, its rows opening what they name where the library can.
fn ranking(title: &str, kind: Kind, entries: &[RecapEntry]) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .child(
            RankedList::new(t(title))
                .columns(1)
                .rows(entries.iter().map(|entry| row(kind, entry))),
        )
        .into_any_element()
}

#[derive(Clone, Copy)]
enum Kind {
    Track,
    Artist,
    Album,
}

fn row(kind: Kind, entry: &RecapEntry) -> TileRow {
    let (prefix, placeholder) = match kind {
        Kind::Track => ("recap-track", icons::MUSIC),
        Kind::Artist => ("recap-artist", icons::MIC),
        Kind::Album => ("recap-album", icons::DISC),
    };
    let identity = entry
        .key
        .clone()
        .unwrap_or_else(|| format!("{}\u{1f}{}", entry.title, entry.subtitle));
    let row = TileRow::new(format!("{prefix}:{identity}"), entry.title.clone())
        .subtitle(entry.subtitle.clone())
        .artwork(entry.artwork.as_ref())
        .placeholder(placeholder)
        .round(matches!(kind, Kind::Artist))
        .detail(t_with("home_plays", &[("count", entry.plays.into())]));
    let route = match (kind, entry.key.clone()) {
        (Kind::Album, Some(id)) => Some(Route::Album(id.into())),
        (Kind::Artist, Some(key)) => Some(Route::Artist(key)),
        _ => None,
    };
    match route {
        Some(route) => row.route(route),
        None => row,
    }
}

impl Render for Recap {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.recap.read(cx);
        let body = match (query.value().cloned(), query.error().cloned()) {
            (Some(recap), _) => self.loaded(&recap, cx),
            (None, Some(error)) => {
                let page = cx.entity().downgrade();
                crate::states::failed(&error, move |_, _, cx| {
                    page.update(cx, |this, cx| {
                        this.recap.update(cx, |query, cx| query.refresh(cx))
                    })
                    .ok();
                })
                .into_any_element()
            }
            (None, None) => Loading::new().into_any_element(),
        };
        v_flex()
            .id("recap-scroll")
            .flex_1()
            .min_h_0()
            .w_full()
            .gap_6()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pb_32()
            .child(self.period_picker(cx))
            .child(body)
    }
}

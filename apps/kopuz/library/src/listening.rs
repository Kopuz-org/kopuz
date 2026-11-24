use api::{ListeningStats, Ranked};
use components::format::{t, t_with};
use components::style;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, IntoElement, ParentElement as _, Render, SharedString, Styled, Task, Window, div,
    px,
};
use store::Link;

const RANGES: [(Option<u32>, &str); 4] = [
    (Some(7), "listening_week"),
    (Some(30), "listening_month"),
    (Some(365), "listening_year"),
    (None, "listening_all_time"),
];
const CHART_HEIGHT: f32 = 64.;

pub struct ListeningPanel {
    days: Option<u32>,
    stats: Option<ListeningStats>,
    /// Why the last read failed, shown while there are no stats to draw.
    failed: Option<SharedString>,
    _load: Option<Task<()>>,
}

impl ListeningPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            days: Some(30),
            stats: None,
            failed: None,
            _load: None,
        };
        this.load(cx);
        this
    }

    fn pick(&mut self, days: Option<u32>, cx: &mut Context<Self>) {
        if self.days != days {
            self.days = days;
            self.load(cx);
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let days = self.days;
        let call = Link::global(cx).call(move |api| async move { api.listening_stats(days).await });
        self._load = Some(cx.spawn(async move |this, cx| {
            let stats = call.await;
            this.update(cx, |this, cx| {
                match stats {
                    Ok(stats) => {
                        this.stats = Some(stats);
                        this.failed = None;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "could not read the listening stats");
                        this.stats = None;
                        this.failed = Some(store::human(&error));
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The span picker, a segmented control so the arrows and Tab reach it like any other.
    fn ranges(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let panel = cx.entity().downgrade();
        components::Segmented::new("listening-range")
            .segments(
                RANGES
                    .iter()
                    .map(|(_, label)| components::Segment::new(*label).label(t(label))),
            )
            .selected(RANGES.iter().position(|(days, _)| *days == self.days))
            .on_select(move |index, _, cx| {
                let Some((days, _)) = RANGES.get(*index) else {
                    return;
                };
                let days = *days;
                panel.update(cx, |this, cx| this.pick(days, cx)).ok();
            })
    }

    fn chart(stats: &ListeningStats, days: Option<u32>, cx: &App) -> impl IntoElement + use<> {
        let bars = bars(stats, days, today());
        let most = bars.iter().copied().max().unwrap_or(1).max(1);
        h_flex()
            .w_full()
            .h(px(CHART_HEIGHT))
            .items_end()
            .gap(px(2.))
            .children(bars.into_iter().map(|plays| {
                let share = plays as f32 / most as f32;
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px((CHART_HEIGHT * share).max(2.)))
                    .rounded_t(px(2.))
                    .map(|bar| match plays {
                        0 => bar.bg(style::white(style::HOVER, cx)),
                        _ => bar.bg(cx.theme().primary),
                    })
            }))
    }

    fn top(title: SharedString, rows: &[Ranked], cx: &App) -> impl IntoElement + use<> {
        v_flex()
            .flex_1()
            .min_w(px(220.))
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(style::white(style::TEXT_SECONDARY, cx))
                    .child(title),
            )
            .children(rows.iter().take(5).enumerate().map(|(index, row)| {
                h_flex()
                    .gap_2()
                    .text_sm()
                    .child(
                        div()
                            .w(px(16.))
                            .text_color(style::slate_400(cx))
                            .child(SharedString::from((index + 1).to_string())),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().child(row.name.clone()))
                            .when(!row.detail.is_empty(), |cell| {
                                cell.child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .text_color(style::slate_400(cx))
                                        .child(row.detail.clone()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(style::slate_400(cx))
                            .child(t_with("home_plays", &[("count", row.plays.into())])),
                    )
            }))
    }
}

impl Render for ListeningPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ranges = self.ranges(cx);
        let body = self.stats.as_ref().map(|stats| {
            let days = self.days;
            v_flex()
                .gap_4()
                .child(
                    h_flex()
                        .gap_6()
                        .child(total(
                            t_with("home_plays", &[("count", stats.plays.into())]),
                            cx,
                        ))
                        .child(total(time(stats.seconds), cx)),
                )
                .when(!stats.days.is_empty(), |body| {
                    body.child(Self::chart(stats, days, cx))
                })
                .when(stats.plays == 0, |body| {
                    body.child(
                        div()
                            .text_sm()
                            .text_color(style::slate_400(cx))
                            .child(t("listening_nothing_yet")),
                    )
                })
                .when(stats.plays > 0, |body| {
                    body.child(
                        h_flex()
                            .flex_wrap()
                            .items_start()
                            .gap_6()
                            .child(Self::top(t("listening_top_artists"), &stats.artists, cx))
                            .child(Self::top(t("listening_top_albums"), &stats.albums, cx)),
                    )
                })
        });
        v_flex()
            .w_full()
            .gap_3()
            .p_4()
            .rounded(style::radius_xl())
            .border_1()
            .border_color(style::white(style::BORDER_SOFT, cx))
            .bg(style::white(style::RAISED, cx))
            .child(ranges)
            .children(self.failed.clone().map(|reason| {
                components::EmptyState::failed(
                    t("library_read_failed"),
                    cx.listener(|this, _, _, cx| this.load(cx)),
                )
                .detail(reason)
                .compact()
            }))
            .children(body)
    }
}

fn total(text: SharedString, cx: &App) -> impl IntoElement {
    div()
        .text_lg()
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .text_color(cx.theme().foreground)
        .child(text)
}

const MAX_BARS: i64 = 60;

fn today() -> i64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (since.as_secs() / 86_400) as i64
}

fn bars(stats: &ListeningStats, days: Option<u32>, today: i64) -> Vec<u32> {
    let first = match days {
        Some(days) => today - i64::from(days) + 1,
        None => stats.days.iter().map(|day| day.day).min().unwrap_or(today),
    };
    let span = (today - first + 1).max(1);
    let per = (span + MAX_BARS - 1) / MAX_BARS;
    let mut bars = vec![0; ((span + per - 1) / per) as usize];
    let inside = stats
        .days
        .iter()
        .filter(|day| (first..=today).contains(&day.day));
    for day in inside {
        if let Some(bar) = bars.get_mut(((day.day - first) / per) as usize) {
            *bar += day.plays;
        }
    }
    bars
}

fn time(seconds: u64) -> SharedString {
    match seconds < 3600 {
        true => t_with("listening_minutes", &[("count", (seconds / 60).into())]),
        false => t_with(
            "listening_hours",
            &[("hours", format!("{:.1}", seconds as f64 / 3600.).into())],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::DayPlays;

    fn stats(days: &[(i64, u32)]) -> ListeningStats {
        ListeningStats {
            days: days
                .iter()
                .map(|(day, plays)| DayPlays {
                    day: *day,
                    plays: *plays,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_month_draws_every_day_quiet_ones_too() {
        let bars = bars(&stats(&[(100, 2), (129, 3)]), Some(30), 129);
        assert_eq!(bars.len(), 30);
        assert_eq!(bars[0], 2);
        assert_eq!(bars[29], 3);
        assert_eq!(bars[1..29].iter().sum::<u32>(), 0);
    }

    #[test]
    fn a_year_folds_days_into_at_most_sixty_bars() {
        let bars = bars(&stats(&[(0, 1), (364, 1)]), Some(365), 364);
        assert!(bars.len() <= MAX_BARS as usize);
        assert_eq!(bars.iter().sum::<u32>(), 2);
    }

    #[test]
    fn days_outside_the_span_are_left_out() {
        let bars = bars(&stats(&[(10, 5), (50, 1)]), Some(7), 50);
        assert_eq!(bars.iter().sum::<u32>(), 1);
    }
}

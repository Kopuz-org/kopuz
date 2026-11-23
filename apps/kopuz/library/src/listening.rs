//! Listening over a span: totals, plays per day, and who and what came up most.

use api::{ListeningStats, Ranked};
use components::format::{t, t_with};
use components::style;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Task, Window, div, px,
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
    _load: Option<Task<()>>,
}

impl ListeningPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            days: Some(30),
            stats: None,
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
                    Ok(stats) => this.stats = Some(stats),
                    Err(error) => tracing::debug!(%error, "no listening stats"),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn ranges(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        h_flex()
            .gap_1()
            .children(RANGES.iter().map(|(days, label)| {
                let days = *days;
                let on = self.days == days;
                div()
                    .id(SharedString::from(format!("listening-range-{label}")))
                    .px_3()
                    .py_1()
                    .rounded_full()
                    .cursor_pointer()
                    .text_xs()
                    .map(|chip| match on {
                        true => chip
                            .bg(style::white(style::HOVER, cx))
                            .text_color(cx.theme().foreground),
                        false => chip.text_color(style::slate_400(cx)),
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.pick(days, cx)))
                    .child(t(label))
            }))
    }

    fn chart(stats: &ListeningStats, cx: &App) -> impl IntoElement + use<> {
        let most = stats
            .days
            .iter()
            .map(|day| day.plays)
            .max()
            .unwrap_or(1)
            .max(1);
        h_flex()
            .w_full()
            .h(px(CHART_HEIGHT))
            .items_end()
            .gap(px(2.))
            .children(stats.days.iter().map(|day| {
                let share = day.plays as f32 / most as f32;
                div()
                    .flex_1()
                    .min_w(px(2.))
                    .max_w(px(18.))
                    .h(px((CHART_HEIGHT * share).max(2.)))
                    .rounded_t(px(2.))
                    .bg(cx.theme().primary)
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
                            .child(t_with("listening_plays", &[("count", row.plays.into())])),
                    )
            }))
    }
}

impl Render for ListeningPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ranges = self.ranges(cx);
        let body = self.stats.as_ref().map(|stats| {
            let hours = stats.seconds as f64 / 3600.;
            v_flex()
                .gap_4()
                .child(
                    h_flex()
                        .gap_6()
                        .child(total(
                            t_with("listening_plays", &[("count", stats.plays.into())]),
                            cx,
                        ))
                        .child(total(
                            t_with(
                                "listening_hours",
                                &[("hours", format!("{hours:.1}").into())],
                            ),
                            cx,
                        )),
                )
                .when(!stats.days.is_empty(), |body| {
                    body.child(Self::chart(stats, cx))
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

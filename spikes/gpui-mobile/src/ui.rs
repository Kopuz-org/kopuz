//! The one screen: a header (status + Play/Pause) over a touch-scrolled
//! `uniform_list` of tracks. Tapping a row replaces the queue from that row.

use std::ops::Range;

use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{
    Context, SharedString, Subscription, Task, Window, div, prelude::*, px, rgb, uniform_list,
};

use crate::daemon::{Daemon, Msg, Row};

const ROW_HEIGHT: f32 = 64.;

pub struct Library {
    daemon: Daemon,
    status: SharedString,
    rows: Vec<Row>,
    keys: Vec<String>,
    playing: bool,
    selected: Option<usize>,
    /// Logical px kept clear at the top for the status bar.
    top_inset: f32,
    _subscriptions: Vec<Subscription>,
    _pump: Task<()>,
}

/// "SPIKE scale=<f> size=<w>x<h>", in logical pixels.
pub fn log_size(window: &Window) {
    let size = window.viewport_size();
    log::info!(
        "SPIKE scale={} size={}x{}",
        window.scale_factor(),
        f32::from(size.width).round(),
        f32::from(size.height).round()
    );
}

impl Library {
    pub fn new(
        daemon: Daemon,
        mut rx: UnboundedReceiver<Msg>,
        top_inset: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        log_size(window);
        let subscriptions = vec![
            cx.observe_window_bounds(window, |_, window, _| log_size(window)),
            cx.observe_window_activation(window, |_, window, _| {
                let event = if window.is_window_active() {
                    "resume"
                } else {
                    "pause"
                };
                log::info!("SPIKE lifecycle {event}");
            }),
            cx.observe_window_visibility(window, |_, visibility, _, _| {
                log::info!("SPIKE lifecycle visibility={visibility:?}");
            }),
            cx.observe_window_appearance(window, |_, window, _| {
                log::info!("SPIKE lifecycle appearance={:?}", window.appearance());
            }),
            cx.on_app_quit(|_, _| {
                log::info!("SPIKE lifecycle quit");
                async {}
            }),
        ];

        // Results from the tokio runtime arrive here, on the GPUI thread.
        let pump = cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let alive = this.update(cx, |this, cx| {
                    this.apply(msg);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        daemon.load();
        Self {
            daemon,
            status: "connecting…".into(),
            rows: Vec::new(),
            keys: Vec::new(),
            playing: false,
            selected: None,
            top_inset,
            _subscriptions: subscriptions,
            _pump: pump,
        }
    }

    fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Connecting(1) => self.status = "connecting…".into(),
            Msg::Connecting(attempt) => self.status = format!("connecting… (try {attempt})").into(),
            Msg::Connected => self.status = "connected, loading tracks…".into(),
            Msg::Tracks(rows) => {
                self.status = format!("{} tracks", rows.len()).into();
                self.keys = rows.iter().map(|row| row.key.clone()).collect();
                self.rows = rows;
            }
            Msg::Playing(playing) => self.playing = playing,
            Msg::Error(text) => self.status = text.into(),
        }
    }

    fn row(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let row = &self.rows[index];
        div()
            .id(("row", index))
            .w_full()
            .h(px(ROW_HEIGHT))
            .px(px(16.))
            .flex()
            .flex_col()
            .justify_center()
            .border_b_1()
            .border_color(rgb(0x2a2a2a))
            .when(self.selected == Some(index), |row| row.bg(rgb(0x22305a)))
            .child(div().w_full().truncate().child(row.title.clone()))
            .child(
                div()
                    .w_full()
                    .truncate()
                    .text_sm()
                    .text_color(rgb(0xa0a0a0))
                    .child(row.artist.clone()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = Some(index);
                this.daemon.play(this.keys.clone(), index);
                cx.notify();
            }))
    }
}

impl Render for Library {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.))
            .px(px(16.))
            .py(px(12.))
            .bg(rgb(0x1e1e1e))
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .truncate()
                    .child(self.status.clone()),
            )
            .child(
                div()
                    .id("toggle")
                    .px(px(20.))
                    .py(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(0x3355ff))
                    .child(if self.playing { "Pause" } else { "Play" })
                    .on_click(cx.listener(|this, _, _, _| this.daemon.toggle())),
            );

        let list = uniform_list(
            "tracks",
            self.rows.len(),
            cx.processor(|this, range: Range<usize>, _window, cx| {
                range.map(|index| this.row(index, cx)).collect::<Vec<_>>()
            }),
        )
        .flex_1()
        .w_full();

        div()
            .size_full()
            .flex()
            .flex_col()
            .pt(px(self.top_inset))
            .bg(rgb(0x121212))
            .text_color(rgb(0xeeeeee))
            .child(header)
            .child(list)
    }
}

//! A radio station as a card, in the selected list or the directory, with the stream buttons that play it.

use std::rc::Rc;

use api::RadioStationInfo;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled, Window, div, px,
    rems,
};

use crate::format::t;
use crate::{Badge, BadgeTone, Chip, ChipSize, ChipTone, Cover, icons, motion, style};

// `w-11 h-11` station art, and `rounded-md` on it; `Cover` only takes pixels.
const ART: Pixels = px(44.);
const CARD_HOVER: u64 = 0;
const CARD_WASH: u64 = 1;

type OnStream = Rc<dyn Fn(&str, &mut Window, &mut App)>;
type OnAct = Rc<dyn Fn(&mut Window, &mut App)>;
type OnExpand = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// The stream a click plays when the row does not name one.
fn first_stream(station: &RadioStationInfo) -> String {
    station
        .streams
        .first()
        .map(|stream| stream.id.clone())
        .unwrap_or_default()
}

/// `px-3 py-1 rounded-lg text-xs font-medium bg-white/10 text-slate-300`, inside a row that plays.
fn chip(id: SharedString) -> Chip {
    Chip::new(id)
        .tone(ChipTone::Filter)
        .size(ChipSize::Medium)
        .stop_propagation(true)
}

fn stream_chip(station: &str, stream: &str, label: SharedString, play: OnStream) -> Chip {
    let stream_id = stream.to_string();
    chip(SharedString::from(format!("stream-{station}-{stream}")))
        .label(label)
        .on_click(move |_, window, cx| play(&stream_id, window, cx))
}

fn play_button(station: &RadioStationInfo, play: OnStream) -> Button {
    let first = first_stream(station);
    Button::new(SharedString::from(format!("play-{}", station.id)))
        .ghost()
        .icon(icons::icon(icons::PLAY))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            play(&first, window, cx);
        })
}

fn tags(station: &RadioStationInfo) -> Vec<AnyElement> {
    station
        .tags
        .iter()
        .take(2)
        .map(|tag| {
            Badge::new(tag.clone())
                .tone(BadgeTone::Neutral)
                .size(ChipSize::Medium)
                .into_any_element()
        })
        .collect()
}

/// One button per stream while there are few, the first plus a "+N" expander past that.
#[derive(IntoElement)]
pub struct StreamPicker {
    station: RadioStationInfo,
    expanded: bool,
    on_play: Option<OnStream>,
    on_expand: Option<OnExpand>,
}

impl StreamPicker {
    pub fn new(station: &RadioStationInfo) -> Self {
        Self {
            station: station.clone(),
            expanded: false,
            on_play: None,
            on_expand: None,
        }
    }

    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    /// Plays the stream it is given the id of; an empty id means the station's own first.
    pub fn on_play(mut self, play: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_play = Some(Rc::new(play));
        self
    }

    /// Asked to open or close the full list of streams.
    pub fn on_expand(mut self, expand: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_expand = Some(Rc::new(expand));
        self
    }
}

impl RenderOnce for StreamPicker {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let station = self.station;
        let play: OnStream = self.on_play.unwrap_or_else(|| Rc::new(|_, _, _| {}));
        let expand = self.on_expand;
        let streams = &station.streams;
        let buttons: Vec<AnyElement> = match streams.len() {
            0 | 1 => vec![play_button(&station, play).into_any_element()],
            2 => streams
                .iter()
                .map(|stream| {
                    stream_chip(&station.id, &stream.id, t(&stream.name), play.clone())
                        .into_any_element()
                })
                .collect(),
            _ if self.expanded => vec![
                chip(SharedString::from(format!("less-{}", station.id)))
                    .icon(icons::CHEVRON_UP)
                    .on_click(move |_, window, cx| {
                        if let Some(expand) = &expand {
                            expand(false, window, cx);
                        }
                    })
                    .into_any_element(),
            ],
            count => {
                let mut buttons = Vec::new();
                if let Some(first) = streams.first() {
                    buttons.push(
                        stream_chip(&station.id, &first.id, t(&first.name), play.clone())
                            .into_any_element(),
                    );
                }
                buttons.push(
                    chip(SharedString::from(format!("more-{}", station.id)))
                        .label(format!("+{}", count - 1))
                        .on_click(move |_, window, cx| {
                            if let Some(expand) = &expand {
                                expand(true, window, cx);
                            }
                        })
                        .into_any_element(),
                );
                buttons
            }
        };
        h_flex().gap_2().justify_end().children(buttons)
    }
}

/// Which list the station sits in, which decides how it is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StationVariant {
    /// A row of the selected table: the whole row plays, and the buttons pick a stream.
    #[default]
    Pinned,
    /// A directory row: play it, or pin it into the selected table.
    Directory,
}

/// A station card; the whole card plays the first stream.
#[derive(IntoElement)]
pub struct StationRow {
    station: RadioStationInfo,
    variant: StationVariant,
    pinned: bool,
    on_play: Option<OnStream>,
    on_pin: Option<OnAct>,
    on_resize: Option<OnAct>,
}

impl StationRow {
    pub fn new(station: &RadioStationInfo) -> Self {
        Self {
            station: station.clone(),
            variant: StationVariant::Pinned,
            pinned: false,
            on_play: None,
            on_pin: None,
            on_resize: None,
        }
    }

    pub fn variant(mut self, variant: StationVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Whether the station is in the selected list, which a directory row shows as a check.
    pub fn pinned(mut self, pinned: bool) -> Self {
        self.pinned = pinned;
        self
    }

    pub fn on_play(mut self, play: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_play = Some(Rc::new(play));
        self
    }

    pub fn on_pin(mut self, pin: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_pin = Some(Rc::new(pin));
        self
    }

    /// Called once the row has opened or closed its stream list, for a virtualized list to measure again.
    pub fn on_resize(mut self, resize: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_resize = Some(Rc::new(resize));
        self
    }
}

impl RenderOnce for StationRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let station = self.station;
        let play: OnStream = self.on_play.unwrap_or_else(|| Rc::new(|_, _, _| {}));
        let muted = style::slate_400(cx);
        let id = match self.variant {
            StationVariant::Pinned => SharedString::from(station.id.clone()),
            StationVariant::Directory => SharedString::from(format!("directory-{}", station.id)),
        };
        // `p-4 rounded-xl border border-white/8 transition-colors hover:bg-white/10`.
        let (hovered, pointer) = motion::hovered(motion::channel(&id, CARD_HOVER), window, cx);
        let wash = motion::color(
            motion::channel(&id, CARD_WASH),
            style::white(if hovered { style::HOVER } else { 0. }, cx),
            window,
            cx,
        );
        let card = div()
            .id(id)
            .w_full()
            .p_4()
            .rounded(style::radius_xl())
            .border_1()
            .border_color(style::white(0.08, cx))
            .cursor_pointer()
            .bg(wash)
            .on_hover(motion::track(pointer))
            .on_click({
                let (play, first) = (play.clone(), first_stream(&station));
                move |_, window, cx| play(&first, window, cx)
            });
        match self.variant {
            StationVariant::Pinned => {
                let expanded: Entity<bool> = window.use_keyed_state(
                    SharedString::from(format!("streams-expanded-{}", station.id)),
                    cx,
                    |_, _| false,
                );
                let open = *expanded.read(cx);
                let on_resize = self.on_resize;
                let toggle = {
                    let expanded = expanded.clone();
                    move |open: bool, window: &mut Window, cx: &mut App| {
                        expanded.update(cx, |held, cx| {
                            *held = open;
                            cx.notify();
                        });
                        if let Some(resize) = &on_resize {
                            resize(window, cx);
                        }
                    }
                };
                let picker = |expanded: bool| {
                    StreamPicker::new(&station)
                        .expanded(expanded)
                        .on_play({
                            let play = play.clone();
                            move |stream, window, cx| play(stream, window, cx)
                        })
                        .on_expand(toggle.clone())
                };
                card.flex()
                    .flex_col()
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_4()
                            .child(
                                Cover::new(station.artwork.as_ref())
                                    .size(ART)
                                    .rounded(style::thumb_radius())
                                    .placeholder(icons::RADIO),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t(&station.name)),
                            )
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_xs()
                                            .text_color(muted)
                                            .child(t(&station.description)),
                                    )
                                    .children(tags(&station)),
                            )
                            .child(
                                // The width the streams column reserves on the right of a row.
                                div().w(rems(13.75)).flex_shrink_0().child(picker(open)),
                            ),
                    )
                    .when(open, |row| {
                        row.child(h_flex().w_full().flex_wrap().gap_2().mt_2p5().children(
                            station.streams.iter().map(|stream| {
                                stream_chip(&station.id, &stream.id, t(&stream.name), play.clone())
                            }),
                        ))
                    })
                    .into_any_element()
            }
            StationVariant::Directory => {
                let pinned = self.pinned;
                let on_pin = self.on_pin;
                // The pin's answer is a round trip away, so the check mark lights as it arrives.
                let pin_tint = motion::color(
                    SharedString::from(format!("pin-tint-{}", station.id)),
                    if pinned {
                        cx.theme().success
                    } else {
                        style::slate_400(cx)
                    },
                    window,
                    cx,
                );
                card.flex()
                    .items_start()
                    .gap_4()
                    .child(
                        Cover::new(station.artwork.as_ref())
                            .size(ART)
                            .rounded(style::thumb_radius())
                            .placeholder(icons::RADIO),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .truncate()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t(&station.name)),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(t(&station.description)),
                            )
                            .child(h_flex().gap_2().mt_2().children(tags(&station))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("pin-{}", station.id)))
                            .custom(style::quiet_button(pin_tint, cx))
                            .rounded(px(999.))
                            .with_size(px(36.))
                            .icon(
                                icons::icon(if pinned { icons::CHECK } else { icons::PLUS })
                                    .xsmall(),
                            )
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                if let Some(pin) = &on_pin {
                                    pin(window, cx);
                                }
                            }),
                    )
                    .child(play_button(&station, play))
                    .into_any_element()
            }
        }
    }
}

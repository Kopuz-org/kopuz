//! The settings card and the rows inside it, as `docs/STYLING.md` maps `.settings-section`.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ElementId, Entity, FontWeight, Global, Hsla, InteractiveElement as _,
    IntoElement, ParentElement, Rems, RenderOnce, SharedString, StatefulInteractiveElement as _,
    Styled, Window, div, px, rems,
};
use store::layout::Ordered;
use store::{Layout, LayoutModel, SettingsFilter};

use crate::format::t;
use crate::{ThinSlider, icons, motion, skin, style};

/// Mark one row by its title; the next jump, or typing in the filter, replaces it.
pub fn set_highlight(title: Option<SharedString>, cx: &mut App) {
    SettingsFilter::global(cx).update(cx, |filter, cx| filter.set_highlight(title, cx));
}

/// Which row is marked, if any.
pub fn highlighted(cx: &App) -> Option<SharedString> {
    SettingsFilter::global(cx).read(cx).highlight().cloned()
}

/// Filter every settings row by `query`; an empty one puts them all back.
pub fn set_query(query: &str, cx: &mut App) {
    SettingsFilter::global(cx).update(cx, |filter, cx| filter.set_query(query, cx));
}

/// What Settings is filtering by, lowercased, or nothing when the box is empty.
pub fn query(cx: &App) -> Option<SharedString> {
    SettingsFilter::global(cx).read(cx).query().cloned()
}

/// Whether a row carrying this text survives the filter. Everything survives an empty box.
pub fn matches(fields: &[&str], cx: &App) -> bool {
    SettingsFilter::global(cx).read(cx).matches(fields)
}

/// True only once a filtered pass has drawn: before that, an empty page means "not drawn yet".
pub fn nothing_matched(cx: &App) -> bool {
    SettingsFilter::global(cx).read(cx).nothing_matched()
}

/// Content that is not a searchable row, so a filtered page leaves it out.
pub fn when_unfiltered<E: IntoElement>(element: E, cx: &App) -> Option<E> {
    query(cx).is_none().then_some(element)
}

/// Remember a searchable line, so "nothing matched" can tell itself from "nothing drawn yet".
fn note(text: &SharedString, cx: &App) {
    SettingsFilter::global(cx).read(cx).note(text);
}

/// `w-55`, the width every enumerated setting's trigger shares.
pub fn select_width() -> Rems {
    rems(13.75)
}

/// `w-40`, the track every settings slider is drawn at.
fn slider_width() -> Rems {
    rems(10.)
}

/// `text-white/80`, what a slider's value reads in.
pub const READOUT: f32 = 0.80;

/// A control a managed settings file can pin; locking only ever adds to what the control already had.
pub trait Lockable {
    fn lock(self, locked: bool) -> Self;
}

type Write<T> = Rc<dyn Fn(T, &mut App)>;

/// One control for every enumerated setting: a fixed-width trigger dropping its options, the current one checked.
#[derive(IntoElement)]
pub struct Choice<T: Clone + PartialEq + 'static> {
    id: ElementId,
    current: T,
    options: Vec<(T, SharedString)>,
    locked: bool,
    fill: bool,
    write: Option<Write<T>>,
}

impl<T: Clone + PartialEq + 'static> Choice<T> {
    pub fn new(id: impl Into<ElementId>, current: T, options: Vec<(T, SharedString)>) -> Self {
        Self {
            id: id.into(),
            current,
            options,
            locked: false,
            fill: false,
            write: None,
        }
    }

    /// Every value of a fixed set, each named by `label`.
    pub fn listed(
        id: impl Into<ElementId>,
        current: T,
        values: impl IntoIterator<Item = T>,
        label: impl Fn(T) -> SharedString,
    ) -> Self {
        let options = values
            .into_iter()
            .map(|value| (value.clone(), label(value)))
            .collect();
        Self::new(id, current, options)
    }

    /// Stretch across the space it is given instead of the shared fixed width.
    pub fn fill(mut self, fill: bool) -> Self {
        self.fill = fill;
        self
    }

    /// Where a pick is stored, such as a config key's writer or a layout field.
    pub fn on_change(mut self, write: impl Fn(T, &mut App) + 'static) -> Self {
        self.write = Some(Rc::new(write));
        self
    }
}

/// A [`Choice`] in one call: its id, the current value, every option, and where a pick goes.
pub fn choice<T: Clone + PartialEq + 'static>(
    id: impl Into<ElementId>,
    current: &T,
    options: Vec<(T, SharedString)>,
    write: impl Fn(T, &mut App) + 'static,
) -> Choice<T> {
    Choice::new(id, current.clone(), options).on_change(write)
}

impl<T: Clone + PartialEq + 'static> Lockable for Choice<T> {
    fn lock(mut self, locked: bool) -> Self {
        self.locked |= locked;
        self
    }
}

impl<T: Clone + PartialEq + 'static> RenderOnce for Choice<T> {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let label = self
            .options
            .iter()
            .find(|(value, _)| *value == self.current)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| t("choose"));
        let trigger = Button::new(self.id)
            .outline()
            .justify_between()
            .label(label)
            .map(|trigger| match self.fill {
                true => trigger.flex_1(),
                false => trigger.w(select_width()),
            });
        // A disabled trigger still opens its dropdown, so a locked one gets none.
        if self.locked {
            return trigger.disabled(true).into_any_element();
        }
        let (current, options, write) = (self.current, self.options, self.write);
        trigger
            .icon(icons::icon(icons::CHEVRON_DOWN))
            .dropdown_menu(move |mut menu, _, _| {
                for (value, label) in &options {
                    let item = PopupMenuItem::new(label.clone()).checked(*value == current);
                    menu = menu.item(match write.clone() {
                        Some(write) => {
                            let value = value.clone();
                            item.on_click(move |_, _, cx| write(value.clone(), cx))
                        }
                        None => item,
                    });
                }
                menu
            })
            .into_any_element()
    }
}

/// A slider with its value read out beside it, the way every numeric setting reads.
#[derive(IntoElement)]
pub struct SliderRow {
    state: Entity<SliderState>,
    readout: Option<Rc<dyn Fn(f32) -> SharedString>>,
    locked: bool,
    fill: bool,
}

impl SliderRow {
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            state: state.clone(),
            readout: None,
            locked: false,
            fill: false,
        }
    }

    /// How the slider's current value reads beside it.
    pub fn readout(mut self, format: impl Fn(f32) -> SharedString + 'static) -> Self {
        self.readout = Some(Rc::new(format));
        self
    }

    /// Stretch across the space it is given instead of the shared fixed track.
    pub fn fill(mut self, fill: bool) -> Self {
        self.fill = fill;
        self
    }
}

impl Lockable for SliderRow {
    fn lock(mut self, locked: bool) -> Self {
        self.locked |= locked;
        self
    }
}

impl RenderOnce for SliderRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let (value, filled) = {
            let state = self.state.read(cx);
            let value = state.value().end();
            let span = state.max_value() - state.min_value();
            let filled = match span > 0. {
                true => (value - state.min_value()) / span,
                false => 0.,
            };
            (value, filled)
        };
        let fill = self.fill;
        h_flex()
            .gap_3()
            .items_center()
            .map(|row| match fill {
                true => row.flex_1().min_w_0(),
                false => row.min_w(select_width()),
            })
            .child(
                div()
                    .map(|track| match fill {
                        true => track.flex_1(),
                        false => track.w(slider_width()),
                    })
                    .map(|track| match skin::material() {
                        true => track.child(
                            ThinSlider::new(
                                SharedString::from(format!(
                                    "setting-slider-{}",
                                    self.state.entity_id()
                                )),
                                &self.state,
                            )
                            .filled(filled)
                            .disabled(self.locked),
                        ),
                        false => track.child(Slider::new(&self.state).disabled(self.locked)),
                    }),
            )
            .when_some(self.readout, |row, format| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .w_16()
                        .text_xs()
                        .text_right()
                        .text_color(style::white(READOUT, cx))
                        .child(format(value)),
                )
            })
    }
}

/// A slider's state over `min..=max` in `step`s, starting at `value`.
pub fn slider_state(min: f32, max: f32, step: f32, value: f32) -> SliderState {
    SliderState::new()
        .min(min)
        .max(max)
        .step(step)
        .default_value(value)
}

/// Write a settings slider's value as it moves, rounded to the whole units every such slider steps in.
pub fn on_slider_change<V: 'static>(
    state: &Entity<SliderState>,
    cx: &mut gpui_kit::Context<V>,
    write: impl Fn(f32, &mut App) + 'static,
) -> gpui_kit::Subscription {
    cx.subscribe(state, move |_, _, event: &SliderEvent, cx| {
        if let SliderEvent::Change(value) = event {
            write(value.end().round(), cx);
        }
    })
}

/// Move a slider to a value it is told from outside, leaving it be when it is already there.
pub fn set_slider(state: &Entity<SliderState>, value: f32, window: &mut Window, cx: &mut App) {
    if (state.read(cx).value().end() - value).abs() < f32::EPSILON {
        return;
    }
    state.update(cx, |state, cx| state.set_value(value, window, cx));
}

/// The offered step a saved value reads as, so a hand-edited or dragged one still lights an option.
pub fn nearest(steps: &[f32], value: f32) -> Option<f32> {
    steps.iter().copied().min_by(|a, b| {
        (a - value)
            .abs()
            .partial_cmp(&(b - value).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// A boolean setting's row; `write` stores the answer wherever that setting lives.
pub fn toggle_row(
    id: impl Into<SharedString>,
    title: impl Into<SharedString>,
    on: bool,
    write: impl Fn(bool, &mut App) + 'static,
) -> SettingRow {
    SettingRow::new(title).setting(
        Toggle::new(id)
            .enabled(on)
            .on_change(move |on, _, cx| write(*on, cx)),
    )
}

/// One name of a reorderable list: steps past its neighbours, and a toggle that shows or hides it.
pub fn order_row<L: Ordered + 'static>(
    prefix: &'static str,
    name: &'static str,
    title: impl Into<SharedString>,
    list: &L,
    field: fn(&mut Layout) -> &mut L,
    cx: &App,
) -> SettingRow {
    let shown = list.shows(name);
    let order = list.visible();
    let at = order.iter().position(|held| *held == name);
    let first = at == Some(0);
    let last = at.is_some_and(|at| at + 1 == order.len());
    let step = |delta: isize, glyph: &'static str, enabled: bool| {
        Button::new(SharedString::from(format!("{prefix}-move-{name}-{delta}")))
            .small()
            .custom(style::quiet_button(style::slate_400(cx), cx))
            .icon(icons::icon(glyph))
            .disabled(!enabled)
            .on_click(move |_, _, cx| {
                LayoutModel::edit(cx, |layout| field(layout).step(name, delta))
            })
    };
    SettingRow::new(title).control(
        h_flex()
            .gap_1()
            .items_center()
            .child(step(-1, icons::CHEVRON_UP, shown && !first))
            .child(step(1, icons::CHEVRON_DOWN, shown && !last))
            .child(
                Toggle::new(SharedString::from(format!("{prefix}-shows-{name}")))
                    .enabled(shown)
                    .on_change(move |on, _, cx| {
                        let on = *on;
                        LayoutModel::edit(cx, |layout| field(layout).show(name, on));
                    }),
            ),
    )
}

/// `.settings-section`: `border-white/11` over `bg-black/78`.
const CARD_BORDER: f32 = 0.11;
const CARD_BG: f32 = 0.78;
/// `.settings-section-header`: `bg-black/58`, lifting to `bg-white/7` under the pointer.
const HEADER_BG: f32 = 0.58;
const HEADER_HOVER: f32 = 0.07;
/// `divide-white/[0.07]` between rows.
const DIVIDER: f32 = 0.07;
/// `bg-white/2.5` on every second row.
const STRIPE: f32 = 0.025;
/// `text-white/65` on the section heading.
const HEADING: f32 = 0.65;
/// `text-white/40` on the lock a managed row is marked with.
const LOCK: f32 = 0.40;
/// `hover:text-slate-300` on a toggle's idle half, one step brighter than the muted token.
const TOGGLE_IDLE_HOVER: f32 = 0.70;
/// `opacity-50` on a locked row's control.
const LOCKED_OPACITY: f32 = 0.50;
/// The accent wash the palette leaves on the row it sent someone to.
const MARKED: f32 = 0.14;

/// Settings cards and the category rail share a translucent theme surface over ambient light.
pub fn panel_background(opacity: f32, cx: &App) -> Hsla {
    if theme::ambient(cx).is_some() {
        cx.theme().popover.opacity(opacity * 0.6)
    } else {
        style::black(opacity, cx)
    }
}

/// `min-h-[3.25rem]`, the height a settings row never goes under.
pub fn row_height() -> Rems {
    rems(3.25)
}

/// `prefix` over a domain key: one stable channel per row, per section, per toggle half.
fn scoped(prefix: &'static str, key: &SharedString) -> ElementId {
    (ElementId::from(prefix), key.clone()).into()
}

/// A titled group of rows: the card the Dioxus settings page draws one category on.
pub fn section(title: impl Into<SharedString>) -> Section {
    Section {
        title: title.into(),
        rows: Vec::new(),
        opaque: false,
    }
}

#[derive(IntoElement)]
pub struct Section {
    title: SharedString,
    rows: Vec<AnyElement>,
    opaque: bool,
}

impl Section {
    /// Its contents are cards, not searchable rows, so the filter can only go by this title.
    pub fn opaque(mut self) -> Self {
        self.opaque = true;
        self
    }
}

impl ParentElement for Section {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.rows.extend(elements);
    }
}

impl RenderOnce for Section {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        note(&self.title, cx);
        if query(cx).is_some() {
            let named = matches(&[self.title.as_str()], cx);
            let keep = if self.opaque {
                named
            } else {
                !self.rows.is_empty()
            };
            if !keep {
                return div().into_any_element();
            }
        }
        let searching = query(cx).is_some();
        let open = window.use_keyed_state(scoped("section-open", &self.title), cx, |_, _| true);
        // A filtered pass must not leave a match hidden inside a section that was collapsed earlier.
        let expanded = searching || *open.read(cx);
        let last = self.rows.len().saturating_sub(1);
        let (over, hovered) = motion::hovered(scoped("section-hover", &self.title), window, cx);
        let header_target = if over {
            style::white(HEADER_HOVER, cx)
        } else if theme::ambient(cx).is_some() {
            style::white(STRIPE, cx)
        } else {
            style::black(HEADER_BG, cx)
        };
        let header_bg = motion::color(
            scoped("section-header", &self.title),
            header_target,
            window,
            cx,
        );
        v_flex()
            .w_full()
            .overflow_hidden()
            .rounded(style::radius_xl())
            .border_1()
            .border_color(style::white(CARD_BORDER, cx))
            .bg(panel_background(CARD_BG, cx))
            .shadow_lg()
            .child(
                h_flex()
                    .id(scoped("section", &self.title))
                    .w_full()
                    .gap_3()
                    .px_5()
                    .py_3()
                    .justify_between()
                    .items_center()
                    .bg(header_bg)
                    .cursor_pointer()
                    .on_hover(motion::track(hovered))
                    .on_click(move |_, _, cx| {
                        open.update(cx, |held, cx| {
                            *held = !*held;
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(style::white(HEADING, cx))
                            .child(SharedString::from(self.title.to_uppercase())),
                    )
                    .child(
                        icons::icon(if expanded {
                            icons::CHEVRON_UP
                        } else {
                            icons::CHEVRON_DOWN
                        })
                        .with_size(style::text_11())
                        .text_color(style::white(HEADING, cx)),
                    ),
            )
            .when(expanded, |card| {
                card.child(
                    v_flex()
                        .w_full()
                        .children(self.rows.into_iter().enumerate().map(|(index, row)| {
                            div()
                                .w_full()
                                .when(index % 2 == 1, |band| band.bg(style::white(STRIPE, cx)))
                                .when(index < last, |band| {
                                    band.border_b_1().border_color(style::white(DIVIDER, cx))
                                })
                                .child(row)
                        })),
                )
            })
            .into_any_element()
    }
}

/// Answers what pins a config key, if anything, and whether its write just landed; the app wiring installs it.
struct Binding(fn(&str, &App) -> (Option<SharedString>, bool));

impl Global for Binding {}

/// The layer pinning `key`, cited the way a settings row cites it.
pub(crate) fn lock_cause(key: &str, cx: &App) -> Option<SharedString> {
    cx.try_global::<Binding>()
        .and_then(|binding| (binding.0)(key, cx).0)
}

/// Install how a row learns the state of its key; `commands::init` is the only caller.
pub fn register_binding(resolve: fn(&str, &App) -> (Option<SharedString>, bool), cx: &mut App) {
    cx.set_global(Binding(resolve));
}

/// A row's control, built once the row knows whether it is locked.
type Control = Box<dyn FnOnce(bool) -> AnyElement>;

/// One setting: its label on the left, its control on the right.
#[derive(IntoElement)]
pub struct SettingRow {
    title: SharedString,
    description: Option<SharedString>,
    locked: bool,
    lock_cause: Option<SharedString>,
    stacked: bool,
    saved: bool,
    control: Option<Control>,
}

impl SettingRow {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            description: None,
            locked: false,
            lock_cause: None,
            stacked: false,
            saved: false,
            control: None,
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The debounced write for this key just landed; the row says so and then stops.
    pub fn saved(mut self, saved: bool) -> Self {
        self.saved = saved;
        self
    }

    /// Nothing when the Settings search box is filtering this row out.
    pub fn shown(self, cx: &App) -> Option<Self> {
        let hit = {
            note(&self.title, cx);
            let mut fields = vec![self.title.as_str()];
            if let Some(description) = self.description.as_ref() {
                note(description, cx);
                fields.push(description.as_str());
            }
            matches(&fields, cx)
        };
        hit.then_some(self)
    }

    /// A managed settings file pins this row's key; its control is inert and says so.
    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    /// Tie the row to a config key: locked, citing the layer, when one pins it; ticked once its write lands.
    pub fn bind(mut self, key: &str, cx: &App) -> Self {
        let (cause, saved) = cx
            .try_global::<Binding>()
            .map_or((None, false), |binding| (binding.0)(key, cx));
        self.locked = cause.is_some();
        self.lock_cause = cause;
        self.saved(saved)
    }

    /// Put the control under the label instead of beside it, for a control that needs the width.
    pub fn stacked(mut self) -> Self {
        self.stacked = true;
        self
    }

    /// A control the row's lock does not reach, such as a panel that reads its own.
    pub fn control(mut self, control: impl IntoElement) -> Self {
        let control = control.into_any_element();
        self.control = Some(Box::new(move |_| control));
        self
    }

    /// A control the row locks along with itself.
    pub fn setting(mut self, control: impl Lockable + IntoElement + 'static) -> Self {
        self.control = Some(Box::new(move |locked| {
            control.lock(locked).into_any_element()
        }));
        self
    }
}

impl RenderOnce for SettingRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (over, hovered) = motion::hovered(scoped("row-hover", &self.title), window, cx);
        let marked = highlighted(cx).is_some_and(|held| held == self.title);
        let lift = if over { style::RAISED } else { 0. };
        let resting = match marked {
            true => style::accent(MARKED, cx),
            false => style::white(lift, cx),
        };
        let surface = motion::color(scoped("row-bg", &self.title), resting, window, cx);
        let held = motion::amount(
            scoped("row-lock", &self.title),
            if self.locked { LOCKED_OPACITY } else { 1. },
            motion::FAST,
            window,
            cx,
        );
        let label = v_flex()
            .min_w_0()
            .gap_0p5()
            .child(
                h_flex()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .min_w_0()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(style::white(style::TEXT_STRONG, cx))
                            .child(self.title.clone()),
                    )
                    .when(self.locked, |heading| {
                        let cause = self
                            .lock_cause
                            .clone()
                            .unwrap_or_else(|| t("setting_managed_by_system"));
                        heading.child(
                            div()
                                .id(scoped("row-lock", &self.title))
                                .flex_shrink_0()
                                .tooltip(move |window, cx| {
                                    Tooltip::new(cause.clone()).build(window, cx)
                                })
                                .child(
                                    icons::icon(icons::LOCK)
                                        .with_size(style::text_10())
                                        .text_color(style::white(LOCK, cx)),
                                ),
                        )
                    }),
            )
            .when_some(self.description, |column, description| {
                column.child(
                    div()
                        .text_xs()
                        .text_color(style::slate_400(cx))
                        .child(description),
                )
            });
        let tick = self.saved.then(|| {
            h_flex()
                .gap_1()
                .items_center()
                .text_size(style::text_10())
                .text_color(cx.theme().success)
                .child(icons::icon(icons::CHECK).with_size(style::text_10()))
                .child(t("setting_saved"))
        });
        let locked = self.locked;
        if self.stacked {
            // A stacked control needs the full width, so the saved tick sits on the label's line instead.
            let control = self.control.map(|control| {
                div()
                    .w_full()
                    .when(held < 1., |fading| fading.opacity(held))
                    .child(control(locked))
            });
            return v_flex()
                .id(scoped("row", &self.title))
                .w_full()
                .min_h(row_height())
                .px_5()
                .py_4()
                .bg(surface)
                .on_hover(motion::track(hovered))
                .child(
                    h_flex()
                        .mb_3()
                        .gap_2()
                        .items_start()
                        .justify_between()
                        .child(label)
                        .children(tick),
                )
                .children(control)
                .into_any_element();
        }
        let control = self.control.map(|control| {
            h_flex().gap_2().items_center().children(tick).child(
                div()
                    .when(held < 1., |fading| fading.opacity(held))
                    .child(control(locked)),
            )
        });
        h_flex()
            .id(scoped("row", &self.title))
            .w_full()
            .min_h(row_height())
            .gap_5()
            .px_5()
            .py_2p5()
            .justify_between()
            .items_center()
            .bg(surface)
            .on_hover(motion::track(hovered))
            .child(label)
            .children(control)
            .into_any_element()
    }
}

/// What a toggle answers with; both halves share one handler, so it is counted, not moved.
type Answer = Rc<dyn Fn(&bool, &mut Window, &mut App) + 'static>;

/// `w-48`, the track the thumb slides along.
fn toggle_width() -> Rems {
    rems(12.)
}

/// `inset-inline-start: 4px`, the gap the thumb keeps from the track's ends.
const THUMB_INSET: f32 = 4.;
/// `border`, which the thumb's `50%` resolves against the padding box without.
const TOGGLE_BORDER: f32 = 1.;

/// `ToggleSetting`: the segmented Enabled/Disabled control every boolean setting answers with.
#[derive(IntoElement)]
pub struct Toggle {
    id: SharedString,
    enabled: bool,
    disabled: bool,
    on_change: Option<Answer>,
}

impl Toggle {
    /// `id` is the row's config key, so the two halves keep stable, domain-derived ids.
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            enabled: false,
            disabled: false,
            on_change: None,
        }
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    /// One half of the segmented control: the thumb says which is the answer, so a half only colors.
    fn half(
        &self,
        value: bool,
        label: SharedString,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        let id = SharedString::from(format!("{}-{}", self.id, if value { "on" } else { "off" }));
        let selected = value == self.enabled;
        let (over, hovered) = motion::hovered(scoped("toggle-half-hover", &id), window, cx);
        let target = if selected {
            style::white(1., cx)
        } else if !self.disabled && over {
            style::white(TOGGLE_IDLE_HOVER, cx)
        } else {
            style::slate_500(cx)
        };
        let text = motion::color_over(scoped("toggle-half", &id), target, motion::SLOW, window, cx);
        let half = h_flex()
            .id(id)
            .flex_1()
            .h_8()
            .items_center()
            .justify_center()
            .rounded(style::radius_lg())
            .text_size(style::text_11())
            .font_weight(FontWeight::BOLD)
            .text_color(text)
            .child(label);
        if self.disabled {
            return half;
        }
        let half = half.cursor_pointer().on_hover(motion::track(hovered));
        match self.on_change.clone() {
            Some(handler) => half.on_click(move |_, window, cx| handler(&value, window, cx)),
            None => half,
        }
    }
}

impl Lockable for Toggle {
    fn lock(mut self, locked: bool) -> Self {
        self.disabled |= locked;
        self
    }
}

impl RenderOnce for Toggle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let half = (style::to_px(toggle_width(), window) - px(TOGGLE_BORDER * 2.)) / 2.;
        let resting = if self.enabled {
            px(THUMB_INSET)
        } else {
            half + px(THUMB_INSET / 2.)
        };
        let offset = motion::length(
            scoped("toggle-thumb", &self.id),
            resting,
            motion::SLOW,
            window,
            cx,
        );
        let thumb = div()
            .absolute()
            .left(offset)
            .h_8()
            .w(half - px(THUMB_INSET))
            .rounded(style::radius_lg())
            .bg(style::white(style::HOVER, cx));
        let on = self.half(true, t("enabled"), window, cx);
        let off = self.half(false, t("disabled"), window, cx);
        h_flex()
            .relative()
            .w_48()
            .h_10()
            .p_1()
            .items_center()
            .rounded(style::radius_xl())
            .border_1()
            .border_color(style::white(style::BORDER_SOFT, cx))
            .bg(style::white(style::RAISED, cx))
            .child(thumb)
            .child(on)
            .child(off)
    }
}

#[cfg(test)]
mod tests {
    use super::nearest;

    #[test]
    fn a_saved_value_lights_the_step_it_is_closest_to() {
        let widths = [200., 260., 320., 380.];
        assert_eq!(nearest(&widths, 263.), Some(260.));
        assert_eq!(nearest(&widths, 1000.), Some(380.));
        assert_eq!(nearest(&widths, 0.), Some(200.));
        assert_eq!(nearest(&[0.8, 0.9, 1.0, 1.1], 1.12), Some(1.1));
        assert_eq!(nearest(&[], 1.), None);
    }
}

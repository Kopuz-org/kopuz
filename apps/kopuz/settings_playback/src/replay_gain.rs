//! ReplayGain: which gain applies, and the two gain sliders a drag previews live through `preview_replay_gain`.

use components::format::t;
use components::settings::slider_state;
use config::{PlaybackEdit, ReplayGainMode, ReplayGainSettings, SettingsEdit};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::{App, AppContext as _, Context, Entity, SharedString, Subscription};
use link::Link;
use store::ConfigModel;

const GAIN_MIN_DB: f32 = -15.0;
const GAIN_MAX_DB: f32 = 15.0;
const STEP_DB: f32 = 0.5;
pub(crate) const CONFIG_KEY: &str = "replay_gain";

pub(crate) fn current(cx: &App) -> ReplayGainSettings {
    ConfigModel::global(cx)
        .read(cx)
        .config()
        .playback
        .replay_gain
}

/// Save the settings and let the engine hear them; a `settled` change always previews.
pub(crate) fn apply(settings: ReplayGainSettings, settled: bool, cx: &mut App) {
    ConfigModel::edit(
        cx,
        SettingsEdit::Playback(PlaybackEdit::ReplayGain(settings)),
    );
    if !settled && !crate::preview_due(cx) {
        return;
    }
    Link::global(cx).fire(cx, "preview replay gain", move |api| async move {
        api.preview_replay_gain(settings).await
    });
}

/// Write one field and keep the rest as the config holds them.
pub(crate) fn change(set: impl FnOnce(&mut ReplayGainSettings), settled: bool, cx: &mut App) {
    let mut settings = current(cx);
    set(&mut settings);
    apply(settings, settled, cx);
}

/// A gain slider over ±15 dB in half-decibel steps, writing through `set` as it moves.
pub(crate) fn gain_slider<V: 'static>(
    value: f32,
    set: fn(&mut ReplayGainSettings, f32),
    cx: &mut Context<V>,
) -> (Entity<SliderState>, Subscription) {
    let slider = cx.new(|_| slider_state(GAIN_MIN_DB, GAIN_MAX_DB, STEP_DB, value));
    let subscription = cx.subscribe(&slider, move |_, _, event: &SliderEvent, cx| {
        let (value, settled) = match event {
            SliderEvent::Change(value) => (value.end(), false),
            SliderEvent::Release(value) => (value.end(), true),
        };
        let db = ((value / STEP_DB).round() * STEP_DB).clamp(GAIN_MIN_DB, GAIN_MAX_DB);
        change(|settings| set(settings, db), settled, cx);
    });
    (slider, subscription)
}

pub(crate) fn decibels(db: f32) -> SharedString {
    SharedString::from(format!("{db:+.1} dB"))
}

pub(crate) fn mode_label(mode: ReplayGainMode) -> SharedString {
    t(mode.i18n_key())
}

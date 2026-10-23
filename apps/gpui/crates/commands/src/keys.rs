//! Bindings and the handlers behind them: one keymap generated from the registry, overrides applied.

use std::collections::BTreeMap;

use gpui_kit::{
    App, DummyKeyboardMapper, Global, KeyBinding, KeyBindingContextPredicate, NoAction, actions,
};
use store::Target;

use crate::registry::{all, get};

actions!(
    kopuz_commands,
    [
        NextTrack,
        PreviousTrack,
        SeekForward,
        SeekBackward,
        VolumeUp,
        VolumeDown,
        ToggleMute,
        ToggleLyrics,
        ToggleQueue,
        ToggleFullscreen,
        ToggleMiniPlayer,
        GoForward,
        GoHome,
        GoLibrary,
        GoDiscover,
        GoRadio,
        OpenSettings,
        ShowShortcuts,
    ]
);

/// Bare keys the Shell claims; typing one into a field must type it, so the field declines them.
const SUPPRESS_IN_INPUT: &[&str] = &["m", "l", "q", "f", "/", "?", "left", "right"];
/// Seeking with the arrows is wrong while a group is being walked with them.
const SUPPRESS_IN_LIST: &[&str] = &["left", "right"];
/// Every key context whose own arrows walk a roving group.
const ARROW_GROUPS: &[&str] = &["TrackList", "NavBar", "SettingsRail", "ShortcutList"];

/// Run a command from a keystroke, through whichever window is in front.
fn fire(id: &'static str, cx: &mut App) {
    let Some(command) = get(id) else {
        tracing::warn!(id, "a binding names a command that is not registered");
        return;
    };
    let Some(window) = cx.active_window() else {
        return;
    };
    // A window already being updated refuses a second update, which is why no binding ever fired.
    cx.defer(move |cx| {
        if let Err(error) = window.update(cx, |_, window, cx| {
            command.perform(Target::None, window, cx)
        }) {
            tracing::warn!(%error, id, "a binding could not reach the window");
        }
    });
}

/// The app-wide handlers; they run last, so anything with focus gets the keystroke first.
pub(crate) fn register_handlers(cx: &mut App) {
    cx.on_action(|_: &NextTrack, cx| fire("player.next", cx));
    cx.on_action(|_: &PreviousTrack, cx| fire("player.previous", cx));
    cx.on_action(|_: &SeekForward, cx| fire("player.seek_forward", cx));
    cx.on_action(|_: &SeekBackward, cx| fire("player.seek_backward", cx));
    cx.on_action(|_: &VolumeUp, cx| fire("player.volume_up", cx));
    cx.on_action(|_: &VolumeDown, cx| fire("player.volume_down", cx));
    cx.on_action(|_: &ToggleMute, cx| fire("player.mute", cx));
    cx.on_action(|_: &ToggleLyrics, cx| fire("view.lyrics", cx));
    cx.on_action(|_: &ToggleQueue, cx| fire("view.queue", cx));
    cx.on_action(|_: &ToggleFullscreen, cx| fire("view.fullscreen", cx));
    cx.on_action(|_: &ToggleMiniPlayer, cx| fire("view.mini_player", cx));
    cx.on_action(|_: &GoForward, cx| fire("nav.forward", cx));
    cx.on_action(|_: &GoHome, cx| fire("nav.home", cx));
    cx.on_action(|_: &GoLibrary, cx| fire("nav.library", cx));
    cx.on_action(|_: &GoDiscover, cx| fire("nav.discover", cx));
    cx.on_action(|_: &GoRadio, cx| fire("nav.radio", cx));
    cx.on_action(|_: &OpenSettings, cx| fire("app.settings", cx));
    cx.on_action(|_: &ShowShortcuts, cx| fire("app.shortcuts", cx));
}

fn bind(keystroke: &str, action: Box<dyn gpui_kit::Action>, context: &str) -> Option<KeyBinding> {
    let predicate = match KeyBindingContextPredicate::parse(context) {
        Ok(predicate) => Some(std::rc::Rc::new(predicate)),
        Err(error) => {
            tracing::error!(%error, context, "unparsable key context");
            return None;
        }
    };
    match KeyBinding::load(
        keystroke,
        action,
        predicate,
        false,
        None,
        &DummyKeyboardMapper,
    ) {
        Ok(binding) => Some(binding),
        Err(error) => {
            tracing::error!(%error, keystroke, "unparsable keystroke");
            None
        }
    }
}

/// The overrides the keymap was last installed with.
struct Installed(BTreeMap<String, String>);

impl Global for Installed {}

/// Install the keymap, then again whenever the saved overrides change, here or in another window.
pub fn keymap(cx: &mut App) {
    install(cx);
    cx.observe(&store::LayoutModel::global(cx), |_, cx| {
        let stale = cx.try_global::<Installed>().map(|installed| &installed.0)
            != Some(&store::layout(cx).shortcuts);
        if stale {
            install(cx);
        }
    })
    .detach();
}

/// Every default binding, with `Layout::shortcuts` overriding a command's first keystroke.
fn install(cx: &mut App) {
    let overrides = store::layout(cx).shortcuts.clone();
    let mut bindings = Vec::new();
    for command in all() {
        let Some(action) = command.action else {
            continue;
        };
        let Some(first) = command.keys.first() else {
            continue;
        };
        match overrides.get(command.id) {
            // The vacated default goes dead first: a reinstall appends, and the last binding wins.
            Some(keystroke) => {
                for binding in command.keys.iter().filter(|b| b.keystroke != keystroke) {
                    bindings.extend(bind(binding.keystroke, Box::new(NoAction), binding.context));
                }
                bindings.extend(bind(keystroke, action(), first.context));
            }
            None => {
                for binding in command.keys {
                    bindings.extend(bind(binding.keystroke, action(), binding.context));
                }
            }
        }
    }
    for keystroke in SUPPRESS_IN_INPUT {
        bindings.extend(bind(keystroke, Box::new(NoAction), "Input"));
    }
    for context in ARROW_GROUPS {
        for keystroke in SUPPRESS_IN_LIST {
            bindings.extend(bind(keystroke, Box::new(NoAction), context));
        }
    }
    cx.bind_keys(bindings);
    cx.set_global(Installed(overrides));
}

/// The keystroke a command answers to now, for a row that shows its shortcut.
pub fn keystroke_for(id: &str, cx: &App) -> Option<String> {
    if let Some(keystroke) = store::layout(cx).shortcuts.get(id) {
        return Some(keystroke.clone());
    }
    get(id)
        .and_then(|command| command.keys.first())
        .map(|binding| binding.keystroke.to_string())
}

/// What a command answers to out of the box, so a row can say whether it has been changed.
pub fn default_keystroke(id: &str) -> Option<&'static str> {
    get(id)
        .and_then(|command| command.keys.first())
        .map(|binding| binding.keystroke)
}

/// The key context a command's binding lives in; a rebind may only collide outside it.
pub fn context_for(id: &str) -> Option<&'static str> {
    get(id)
        .and_then(|command| command.keys.first())
        .map(|binding| binding.context)
}

/// The command `keystroke` would take over from inside `id`'s own context, overrides included.
pub fn conflict(id: &str, keystroke: &str, cx: &App) -> Option<&'static crate::Command> {
    let context = context_for(id)?;
    all().iter().find(|held| {
        held.id != id
            && context_for(held.id) == Some(context)
            && keystroke_for(held.id, cx).as_deref() == Some(keystroke)
    })
}

/// Point `id` at `keystroke`, or back at its default when `None`; the keymap follows the layout.
pub fn rebind(id: &str, keystroke: Option<String>, cx: &mut App) {
    let id = id.to_string();
    store::LayoutModel::edit(cx, move |layout| match keystroke.clone() {
        Some(keystroke) => {
            layout.shortcuts.insert(id.clone(), keystroke);
        }
        None => {
            layout.shortcuts.remove(&id);
        }
    });
}

/// A pressed keystroke in the form the keymap parses back, so a recording round-trips.
pub fn written(keystroke: &gpui_kit::Keystroke) -> String {
    let mut said = String::new();
    if keystroke.modifiers.control {
        said.push_str("ctrl-");
    }
    if keystroke.modifiers.alt {
        said.push_str("alt-");
    }
    if keystroke.modifiers.shift {
        said.push_str("shift-");
    }
    if keystroke.modifiers.platform {
        said.push_str("cmd-");
    }
    said.push_str(&keystroke.key);
    said
}

//! What every key does, generated from the registry: the sheet `?` opens and the editor that rebinds one.

mod editor;
mod sheet;

use commands::{Command, Group};
use components::format::t;
use gpui_kit::{App, AppContext as _, SharedString};
use store::layout::Section as Group_;

/// One line of the sheet and one row of the editor: a verb and the key it answers to.
pub(crate) struct Entry {
    pub id: &'static str,
    pub label: SharedString,
    pub keystroke: Option<String>,
}

/// Every group in the order the palette ranks them, with the commands that have a key.
pub(crate) fn groups(cx: &App) -> Vec<(&'static str, Vec<Entry>)> {
    const ORDER: &[(Group, &str)] = &[
        (Group::Playback, "shortcuts_playback"),
        (Group::Queue, "shortcuts_queue"),
        (Group::Library, "library"),
        (Group::Navigate, "palette_navigate"),
        (Group::View, "view"),
        (Group::App, "shortcuts_app"),
    ];
    ORDER
        .iter()
        .filter_map(|(group, title)| {
            let entries: Vec<Entry> = commands::all()
                .iter()
                .filter(|command| command.group == *group && bindable(command))
                .map(|command| entry(command, cx))
                .collect();
            (!entries.is_empty()).then_some((*title, entries))
        })
        .collect()
}

/// Only a command with a default binding can be listed: the rest have no key to show or change.
fn bindable(command: &Command) -> bool {
    !command.keys.is_empty()
}

fn entry(command: &'static Command, cx: &App) -> Entry {
    Entry {
        id: command.id,
        label: command.label(&store::Target::None, cx),
        keystroke: commands::keystroke_for(command.id, cx),
    }
}

/// The title `?` and the settings section share.
pub(crate) fn title() -> SharedString {
    t("keyboard_shortcuts")
}

/// Wire the sheet `?` opens and contribute the editor to Settings → Interface.
pub fn init(cx: &mut App) {
    store::SHORTCUTS.register(|(), window, cx| sheet::open(window, cx), cx);
    settings_interface::register(
        "shortcuts",
        "keyboard_shortcuts",
        50,
        Some(Group_::Shortcuts),
        |window, cx| cx.new(|cx| editor::Shortcuts::new(window, cx)).into(),
        cx,
    );
}

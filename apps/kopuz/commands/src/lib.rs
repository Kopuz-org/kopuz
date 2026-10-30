//! Every verb in the app, once. Menus, the palette, the keymap and the shortcuts sheet project this list.

mod bar;
mod keys;
mod ops;
mod playlist_picker;
mod registry;
mod track_list;

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{Action, App, Context, SharedString, Window};
use store::{Kind, Target};

pub use bar::{play_all, set_offline, set_shuffle, shuffle_play};
pub use keys::{conflict, context_for, default_keystroke, keymap, keystroke_for, rebind, written};
pub use registry::{all, get};
pub use track_list::bind as bind_track_list;

/// Where a command sits in the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Playback,
    Queue,
    Library,
    Navigate,
    View,
    App,
}

/// Which block of a context menu a command lands in; a change of section draws a separator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Act,
    Go,
    Edit,
    Remove,
    /// Reachable by key or palette, never listed in a menu.
    Hidden,
}

/// One default keystroke and the key context it is scoped to.
pub struct Binding {
    pub keystroke: &'static str,
    pub context: &'static str,
}

/// A verb: what it is called, what it applies to, whether it can be done now, and how to do it.
pub struct Command {
    pub id: &'static str,
    /// Every i18n key `label` can return, so the locale check can see them all.
    pub labels: &'static [&'static str],
    pub label: fn(&Target, &App) -> SharedString,
    pub icon: Option<&'static str>,
    pub group: Group,
    pub section: Section,
    pub keys: &'static [Binding],
    /// Paints in `theme.danger` and confirms before it runs.
    pub danger: bool,
    pub applies: &'static [Kind],
    pub available: fn(&Target, &App) -> bool,
    pub run: fn(Target, &mut Window, &mut App),
    /// The action a binding dispatches; `None` means the binding is installed by whoever owns the handler.
    pub action: Option<fn() -> Box<dyn Action>>,
}

impl Command {
    pub fn applies_to(&self, target: &Target) -> bool {
        self.applies.contains(&target.kind())
    }

    pub fn is_available(&self, target: &Target, cx: &App) -> bool {
        self.applies_to(target) && (self.available)(target, cx)
    }

    pub fn label(&self, target: &Target, cx: &App) -> SharedString {
        (self.label)(target, cx)
    }

    /// Do it, asking first when it cannot be undone.
    pub fn perform(&'static self, target: Target, window: &mut Window, cx: &mut App) {
        if !self.danger {
            (self.run)(target, window, cx);
            return;
        }
        let title = self.label(&target, cx);
        let description = target.label();
        let run = self.run;
        components::dialogs::confirm(
            components::dialogs::ConfirmKind::Delete,
            title,
            description,
            move |window, cx| run(target.clone(), window, cx),
            window,
            cx,
        );
    }
}

/// Build the registry's menu hook; call once at startup, after `store::init`.
pub fn init(cx: &mut App) {
    components::menus::register(|target, menu, _window, cx| build_menu(target, menu, cx), cx);
    components::settings::register_binding(
        |key, cx| {
            let config = store::ConfigModel::global(cx).read(cx);
            (
                config
                    .lock_of(key)
                    .map(|by| SharedString::from(store::lock_cause(by))),
                config.just_saved(key),
            )
        },
        cx,
    );
    keys::register_handlers(cx);
}

/// The one way a context or dots menu is built.
pub fn menu(
    target: Target,
    menu: PopupMenu,
    _: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    build_menu(target, menu, cx)
}

fn build_menu(target: Target, mut menu: PopupMenu, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let mut previous: Option<Section> = None;
    for command in all() {
        if command.section == Section::Hidden || !command.is_available(&target, cx) {
            continue;
        }
        if previous.is_some_and(|section| section != command.section) {
            menu = menu.separator();
        }
        previous = Some(command.section);
        menu = menu.item(item(command, &target, cx));
    }
    menu
}

fn item(command: &'static Command, target: &Target, cx: &App) -> PopupMenuItem {
    let target = target.clone();
    components::menus::menu_item(command.label(&target, cx), command.icon)
        // Taking something off disk should not look like the item above it.
        .danger(command.danger)
        .on_click(move |_, window, cx| command.perform(target.clone(), window, cx))
        .into()
}

/// An icon-only button's tooltip: what it does, and the key that does it without the pointer.
pub fn tooltip(id: &str, label: SharedString, cx: &App) -> SharedString {
    match keystroke_for(id, cx) {
        Some(keystroke) => SharedString::from(format!(
            "{label}  {}",
            components::keycap::written(&keystroke)
        )),
        None => label,
    }
}

/// What the palette lists for `target`, in group order.
pub fn for_palette(target: &Target, cx: &App) -> Vec<&'static Command> {
    let mut commands: Vec<&'static Command> = all()
        .iter()
        .filter(|command| command.is_available(target, cx))
        .collect();
    commands.sort_by_key(|command| group_rank(command.group));
    commands
}

/// How many verbs a selection pill spells out before the rest go under its dots.
const SPELLED_OUT: usize = 3;

/// The selection pill over `target`: its first verbs spelled out, the rest in the dots menu.
pub fn selection_bar(
    id: impl Into<SharedString>,
    count: usize,
    target: Target,
    run: impl Fn(&'static Command, Target, &mut Window, &mut App) + 'static,
    cx: &App,
) -> components::SelectionBar {
    let run = std::rc::Rc::new(run);
    let verbs: Vec<components::Verb> = for_palette(&target, cx)
        .into_iter()
        .filter(|command| command.section == Section::Act)
        .filter_map(|command| {
            let icon = command.icon?;
            let run = run.clone();
            let acted = target.clone();
            Some(components::Verb::new(
                command.id,
                icon,
                command.label(&target, cx),
                move |window, cx| run(command, acted.clone(), window, cx),
            ))
        })
        .take(SPELLED_OUT)
        .collect();
    components::SelectionBar::new(id, count)
        .verbs(verbs)
        .target(target)
}

fn group_rank(group: Group) -> u8 {
    match group {
        Group::Playback => 0,
        Group::Queue => 1,
        Group::Library => 2,
        Group::Navigate => 3,
        Group::View => 4,
        Group::App => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Label keys reported upstream but not landed yet; every one must still be absent from `en.ftl`.
    const PENDING: &[&str] = &[];

    #[test]
    fn no_command_id_is_used_twice() {
        let mut seen = HashSet::new();
        for command in all() {
            assert!(
                seen.insert(command.id),
                "duplicate command id {}",
                command.id
            );
        }
    }

    #[test]
    fn no_keystroke_is_bound_twice_in_one_context() {
        let mut seen = HashSet::new();
        for command in all() {
            for binding in command.keys {
                assert!(
                    seen.insert((binding.context, binding.keystroke)),
                    "{} rebinds {} in {}",
                    command.id,
                    binding.keystroke,
                    binding.context
                );
            }
        }
    }

    #[test]
    fn every_label_key_is_translated() {
        for command in all() {
            assert!(
                !command.labels.is_empty(),
                "{} has no label key",
                command.id
            );
            for key in command.labels {
                if PENDING.contains(key) {
                    continue;
                }
                assert_ne!(
                    i18n::t(key),
                    *key,
                    "{} names a key en.ftl does not have: {key}",
                    command.id
                );
            }
        }
    }

    #[test]
    fn every_pending_key_is_still_missing() {
        for key in PENDING {
            assert_eq!(
                i18n::t(key),
                *key,
                "{key} landed in en.ftl; drop it from PENDING"
            );
        }
    }

    #[test]
    fn every_pending_key_is_actually_used() {
        let used: HashSet<&str> = all()
            .iter()
            .flat_map(|command| command.labels.iter().copied())
            .collect();
        for key in PENDING {
            assert!(
                used.contains(key),
                "{key} is pending but no command asks for it"
            );
        }
    }

    #[test]
    fn a_command_only_offers_itself_for_a_kind_it_handles() {
        for command in all() {
            assert!(
                !command.applies.is_empty(),
                "{} applies to nothing",
                command.id
            );
        }
    }
}

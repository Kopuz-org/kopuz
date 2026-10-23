//! The one selection pill: how much is picked, what to do with it, and the way out.

use std::cell::OnceCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Div, FontWeight, IntoElement, ParentElement as _, Rems, RenderOnce,
    SharedString, Styled as _, Window, div, px, rems,
};
use store::Target;

use crate::format::{t, t_with};
use crate::menu_button::{MenuButton, MenuVariant};
use crate::{dialogs, icons, menus, motion, style};

const FADE: u64 = 0;

type Run = Rc<dyn Fn(&mut Window, &mut App)>;

/// One verb the pill spells out: a glyph, what it is called, and what it does to the selection.
pub struct Verb {
    id: SharedString,
    icon: &'static str,
    label: SharedString,
    run: Run,
}

impl Verb {
    pub fn new(
        id: impl Into<SharedString>,
        icon: &'static str,
        label: impl Into<SharedString>,
        run: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            icon,
            label: label.into(),
            run: Rc::new(run),
        }
    }
}

/// The accent pill floating over a listing while anything in it is picked.
#[derive(IntoElement)]
pub struct SelectionBar {
    id: SharedString,
    count: usize,
    verbs: Vec<Verb>,
    target: Option<Target>,
    delete: Option<Run>,
    cancel: Option<Run>,
    lift: Rems,
}

impl SelectionBar {
    pub fn new(id: impl Into<SharedString>, count: usize) -> Self {
        Self {
            id: id.into(),
            count,
            verbs: Vec::new(),
            target: None,
            delete: None,
            cancel: None,
            lift: rems(2.5),
        }
    }

    pub fn verbs(mut self, verbs: impl IntoIterator<Item = Verb>) -> Self {
        self.verbs.extend(verbs);
        self
    }

    /// What the dots menu acts on, and what a delete's confirmation names.
    pub fn target(mut self, target: Target) -> Self {
        self.target = Some(target);
        self
    }

    /// A delete at the end of the verbs, run only once the confirmation is accepted.
    pub fn on_delete(mut self, delete: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.delete = Some(Rc::new(delete));
        self
    }

    pub fn on_cancel(mut self, cancel: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.cancel = Some(Rc::new(cancel));
        self
    }

    /// How far above the listing's bottom edge it floats, clearing whatever docks there.
    pub fn lift(mut self, lift: Rems) -> Self {
        self.lift = lift;
        self
    }
}

impl RenderOnce for SelectionBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let fade = motion::enter(motion::channel(&id, FADE), motion::MEDIUM, window, cx);
        let ink = cx.theme().primary_foreground;
        let rule = style::black(0.2, cx);
        let divider = move || -> Div { div().w(px(1.)).h_5().flex_shrink_0().bg(rule) };
        // Without this variant a Button's hover layer repaints the glyph grey over the accent slab.
        let bare = style::bare_button(ink, cx);
        let verb = |verb_id: SharedString, icon: &'static str, label: SharedString, run: Run| {
            Button::new(SharedString::from(format!("{id}-{verb_id}")))
                .custom(bare)
                .icon(icons::icon(icon))
                .label(label)
                .font_weight(FontWeight::MEDIUM)
                .on_click(move |_, window, cx| run(window, cx))
        };
        let verbs: Vec<AnyElement> = self
            .verbs
            .into_iter()
            .map(|entry| verb(entry.id, entry.icon, entry.label, entry.run).into_any_element())
            .collect();
        let named = self.target.as_ref().map(Target::label).unwrap_or_default();
        let delete = self.delete.map(|delete| {
            let confirm: Run = Rc::new(move |window, cx| {
                let delete = delete.clone();
                dialogs::confirm(
                    dialogs::ConfirmKind::Delete,
                    t("delete"),
                    named.clone(),
                    move |window, cx| delete(window, cx),
                    window,
                    cx,
                )
            });
            verb("delete".into(), icons::TRASH, t("delete"), confirm)
        });
        let more = self.target.map(|target| {
            MenuButton::new(format!("{id}-more"), menus::menu_for(target))
                .variant(MenuVariant::Bare(ink))
        });
        let cancel = self.cancel;
        div()
            .absolute()
            .bottom(self.lift)
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .opacity(fade)
            .child(
                h_flex()
                    .gap_4()
                    .items_center()
                    .px_6()
                    .py_2p5()
                    .rounded(cx.theme().radius_full())
                    .bg(cx.theme().primary)
                    .text_color(ink)
                    .shadow_2xl()
                    .child(
                        div()
                            .pl_2()
                            .font_weight(FontWeight::BOLD)
                            .child(t_with("library_selected", &[("count", self.count.into())])),
                    )
                    .child(divider())
                    .children(verbs)
                    .children(delete)
                    .children(more)
                    .child(divider())
                    .child(
                        Button::new(SharedString::from(format!("{id}-cancel")))
                            .custom(bare)
                            .icon(icons::icon(icons::CLOSE))
                            .tooltip(t("cancel"))
                            .when_some(cancel, |button, cancel| {
                                button.on_click(move |_, window, cx| cancel(window, cx))
                            }),
                    ),
            )
    }
}

/// What is picked in a listing whose rows may not all be loaded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    picked: Vec<String>,
    /// `picked` as one shared slice, built the first time a frame asks and dropped on any change.
    shared: OnceCell<Arc<[String]>>,
    held: HashSet<String>,
    /// The filtered listing entire, rather than the keys that happen to be loaded.
    everything: bool,
    /// Where a shift-click extends from, as an index into the whole listing.
    anchor: Option<usize>,
    /// Where the arrows are, as an index into the whole listing.
    cursor: Option<usize>,
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the whole filtered listing is picked, loaded or not.
    pub fn is_everything(&self) -> bool {
        self.everything
    }

    /// Whether anything at all is picked, which is what raises the selection bar.
    pub fn is_active(&self) -> bool {
        self.everything || !self.picked.is_empty()
    }

    /// How many rows this stands for; `total` is the whole listing's length.
    pub fn count(&self, total: usize) -> usize {
        match self.everything {
            true => total,
            false => self.picked.len(),
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.everything || self.held.contains(id)
    }

    /// The keys that were actually clicked; empty under select-all, which has to be resolved.
    pub fn picked(&self) -> &[String] {
        &self.picked
    }

    /// The same keys, shared: a row of a big selection holds one without copying every key.
    pub fn picked_shared(&self) -> Arc<[String]> {
        self.shared
            .get_or_init(|| self.picked.as_slice().into())
            .clone()
    }

    pub fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    pub fn set_cursor(&mut self, index: Option<usize>) {
        self.cursor = index;
    }

    pub fn clear(&mut self) {
        let cursor = self.cursor;
        *self = Self::default();
        self.cursor = cursor;
    }

    /// Turn select-all into explicit picks from the keys the caller holds in full.
    pub fn resolve(&mut self, keys: impl IntoIterator<Item = String>) {
        if !self.everything {
            return;
        }
        self.everything = false;
        self.shared.take();
        for key in keys {
            if self.held.insert(key.clone()) {
                self.picked.push(key);
            }
        }
    }

    /// Ctrl-click: add or drop one row, and make it where a later shift-click reaches from.
    pub fn toggle(&mut self, id: &str, index: usize) {
        self.shared.take();
        // Select-all cannot become keys without the whole listing, so a plain pick starts over.
        self.forget_everything();
        if self.held.remove(id) {
            self.picked.retain(|held| held != id);
        } else {
            self.held.insert(id.to_string());
            self.picked.push(id.to_string());
            self.anchor = Some(index);
        }
        self.cursor = Some(index);
    }

    /// Shift-click: everything between the last pick and `index`, as far as `rows` can name it.
    pub fn extend_to(&mut self, index: usize, rows: impl Fn(usize) -> Option<String>) {
        self.shared.take();
        let anchor = self.anchor.unwrap_or(index);
        let (first, last) = (anchor.min(index), anchor.max(index));
        self.forget_everything();
        for slot in first..=last {
            let Some(id) = rows(slot) else { continue };
            if self.held.insert(id.clone()) {
                self.picked.push(id);
            }
        }
        self.cursor = Some(index);
    }

    /// Ctrl-A: the whole filtered listing, not the window. Nothing is fetched to say so.
    pub fn select_everything(&mut self) {
        self.shared.take();
        self.picked.clear();
        self.held.clear();
        self.everything = true;
    }

    /// Drop the keys a refetch no longer holds; select-all is scoped to the filter and stays.
    pub fn retain_loaded(&mut self, loaded: &HashSet<String>, complete: bool) {
        self.shared.take();
        if self.everything || !complete {
            return;
        }
        self.picked.retain(|id| loaded.contains(id));
        self.held.retain(|id| loaded.contains(id));
    }

    fn forget_everything(&mut self) {
        if self.everything {
            self.everything = false;
            self.picked.clear();
            self.held.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(index: usize) -> Option<String> {
        (index < 10).then(|| format!("k{index}"))
    }

    #[test]
    fn nothing_picked_raises_no_bar() {
        let selection = Selection::new();
        assert!(!selection.is_active());
        assert_eq!(selection.count(5000), 0);
    }

    #[test]
    fn select_all_stands_for_the_whole_listing_without_holding_a_key() {
        let mut selection = Selection::new();
        selection.select_everything();
        assert!(selection.is_everything());
        assert_eq!(selection.count(40_000), 40_000);
        assert!(selection.picked().is_empty());
        assert!(selection.contains("a track nobody has loaded"));
    }

    #[test]
    fn a_click_after_select_all_starts_a_plain_selection() {
        let mut selection = Selection::new();
        selection.select_everything();
        selection.toggle("k1", 1);
        assert!(!selection.is_everything());
        assert_eq!(selection.picked(), ["k1"]);
        assert_eq!(selection.count(40_000), 1);
    }

    #[test]
    fn a_fully_loaded_list_resolves_select_all_before_a_toggle_drops_one() {
        let mut selection = Selection::new();
        selection.select_everything();
        selection.resolve((0..3).map(|index| format!("k{index}")));
        selection.toggle("k1", 1);
        assert_eq!(selection.picked(), ["k0", "k2"]);
        assert!(!selection.contains("k1"));
    }

    #[test]
    fn a_range_covers_what_the_window_holds() {
        let mut selection = Selection::new();
        selection.toggle("k2", 2);
        selection.extend_to(15, rows);
        assert_eq!(selection.count(100), 8);
        assert!(selection.contains("k9"));
    }

    #[test]
    fn a_refetch_drops_keys_that_are_gone_but_leaves_select_all_alone() {
        let mut selection = Selection::new();
        selection.toggle("k1", 1);
        selection.toggle("k2", 2);
        let loaded = HashSet::from(["k2".to_string()]);
        selection.retain_loaded(&loaded, true);
        assert_eq!(selection.picked(), ["k2"]);

        let mut everything = Selection::new();
        everything.select_everything();
        everything.retain_loaded(&HashSet::new(), true);
        assert!(everything.is_everything());
    }

    #[test]
    fn a_partial_window_never_drops_a_pick_that_is_simply_not_loaded() {
        let mut selection = Selection::new();
        selection.toggle("k1", 1);
        selection.retain_loaded(&HashSet::new(), false);
        assert_eq!(selection.picked(), ["k1"]);
    }

    #[test]
    fn toggling_the_same_row_twice_leaves_nothing_picked() {
        let mut selection = Selection::new();
        selection.toggle("k1", 1);
        selection.toggle("k1", 1);
        assert!(!selection.is_active());
    }

    #[test]
    fn the_shared_keys_follow_every_change_to_the_picks() {
        let mut selection = Selection::new();
        selection.toggle("k1", 1);
        assert_eq!(&*selection.picked_shared(), ["k1"]);
        selection.toggle("k2", 2);
        assert_eq!(&*selection.picked_shared(), ["k1", "k2"]);
        selection.retain_loaded(&HashSet::from(["k2".to_string()]), true);
        assert_eq!(&*selection.picked_shared(), ["k2"]);
    }
}

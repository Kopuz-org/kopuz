//! A library's folders as a removable list, and the in-place browser that picks one off a remote tree.

use std::rc::Rc;

use api::SourceFolderEntry;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, ElementId, IntoElement, ParentElement as _, PathPromptOptions, RenderOnce,
    SharedString, Styled, Window, div, px,
};

use crate::format::t;
use crate::{EmptyState, ErrorBanner, Loading, icons, style};

/// The path one level up, for the browser's back button.
pub fn parent_dir(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(cut) => trimmed[..cut].to_string(),
    }
}

/// Ask the platform for folders and hand `apply` whatever was picked.
pub fn pick_folders(
    multiple: bool,
    cx: &mut App,
    apply: impl FnOnce(Vec<String>, &mut App) + 'static,
) {
    pick_paths(false, true, multiple, cx, apply);
}

/// Ask the platform for files or folders and hand `apply` whatever was picked.
pub fn pick_paths(
    files: bool,
    directories: bool,
    multiple: bool,
    cx: &mut App,
    apply: impl FnOnce(Vec<String>, &mut App) + 'static,
) {
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files,
        directories,
        multiple,
        prompt: None,
    });
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = paths.await else {
            return;
        };
        let picked: Vec<String> = paths
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect();
        if picked.is_empty() {
            return;
        }
        cx.update(|cx| apply(picked, cx));
    })
    .detach();
}

type OnPath = Rc<dyn Fn(&str, &mut Window, &mut App)>;
type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A library's folders, each with a remove button, and the button that adds more.
#[derive(IntoElement)]
pub struct FolderList {
    id: ElementId,
    paths: Vec<String>,
    locked: bool,
    empty: Option<SharedString>,
    on_remove: Option<OnPath>,
    on_add: Option<OnClick>,
}

impl FolderList {
    pub fn new(id: impl Into<ElementId>, paths: Vec<String>) -> Self {
        Self {
            id: id.into(),
            paths,
            locked: false,
            empty: None,
            on_remove: None,
            on_add: None,
        }
    }

    /// A managed setting: nothing can be added or removed.
    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    /// What to say while there are no folders; nothing is said by default.
    pub fn empty(mut self, empty: impl Into<SharedString>) -> Self {
        self.empty = Some(empty.into());
        self
    }

    pub fn on_remove(mut self, remove: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_remove = Some(Rc::new(remove));
        self
    }

    /// Without it the add button is not drawn.
    pub fn on_add(mut self, add: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_add = Some(Rc::new(add));
        self
    }
}

impl RenderOnce for FolderList {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let locked = self.locked;
        let remove = self.on_remove;
        v_flex()
            .w_full()
            .gap_2()
            .when(self.paths.is_empty(), |list| {
                list.children(self.empty.map(|empty| EmptyState::new(empty).compact()))
            })
            .children(self.paths.into_iter().map(|path| {
                let button = Button::new(SharedString::from(format!("{id}-remove-{path}")))
                    .custom(style::tint_text_button(cx.theme().danger, cx))
                    .xsmall()
                    .label(t("remove"))
                    .disabled(locked)
                    .on_click({
                        let remove = remove.clone();
                        let path = path.clone();
                        move |_, window, cx| {
                            if let Some(remove) = &remove {
                                remove(&path, window, cx);
                            }
                        }
                    });
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .p_2()
                    .rounded(style::radius(cx))
                    .bg(style::white(style::RAISED, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(style::slate_400(cx))
                            .child(SharedString::from(path)),
                    )
                    .child(button)
            }))
            .children(self.on_add.map(|add| {
                Button::new(SharedString::from(format!("{id}-add")))
                    .custom(style::soft_button(cx.theme().foreground, cx))
                    .small()
                    .icon(icons::icon(icons::PLUS))
                    .label(t("add_folder"))
                    .disabled(locked)
                    .on_click(move |event, window, cx| add(event, window, cx))
                    .self_start()
            }))
    }
}

/// The folders under `path` on a remote source, to descend into or to take.
#[derive(IntoElement)]
pub struct FolderBrowser {
    path: String,
    entries: Vec<SourceFolderEntry>,
    loading: bool,
    error: Option<SharedString>,
    on_open: Option<OnPath>,
    on_parent: Option<OnClick>,
    on_use: Option<OnPath>,
    on_cancel: Option<OnClick>,
}

impl FolderBrowser {
    pub fn new(path: impl Into<String>, entries: Vec<SourceFolderEntry>) -> Self {
        Self {
            path: path.into(),
            entries,
            loading: false,
            error: None,
            on_open: None,
            on_parent: None,
            on_use: None,
            on_cancel: None,
        }
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn error(mut self, error: Option<SharedString>) -> Self {
        self.error = error;
        self
    }

    /// Descend into the folder at the path it is given.
    pub fn on_open(mut self, open: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_open = Some(Rc::new(open));
        self
    }

    pub fn on_parent(
        mut self,
        parent: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_parent = Some(Rc::new(parent));
        self
    }

    /// Take the folder the browser is showing, handed its path.
    pub fn on_use(mut self, take: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_use = Some(Rc::new(take));
        self
    }

    pub fn on_cancel(
        mut self,
        cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_cancel = Some(Rc::new(cancel));
        self
    }
}

impl RenderOnce for FolderBrowser {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let path = self.path;
        let at_root = path == "/";
        let (on_parent, on_cancel) = (self.on_parent, self.on_cancel);
        let (on_open, on_use) = (self.on_open, self.on_use);
        let listing = v_flex().w_full().gap_1().max_h(px(224.)).map(|list| {
            if self.loading {
                return list.child(Loading::new().label(t("loading_folders")).compact());
            }
            if let Some(error) = self.error {
                return list.child(ErrorBanner::new(error));
            }
            if self.entries.is_empty() {
                return list.child(EmptyState::new(t("no_subfolders")).compact());
            }
            list.children(self.entries.into_iter().map(|entry| {
                let on_open = on_open.clone();
                let target = entry.path.clone();
                Button::new(SharedString::from(format!("browse-{}", entry.path)))
                    .ghost()
                    .xsmall()
                    .justify_start()
                    .w_full()
                    .icon(icons::icon(icons::FOLDER))
                    .label(SharedString::from(entry.name))
                    .on_click(move |_, window, cx| {
                        if let Some(open) = &on_open {
                            open(&target, window, cx);
                        }
                    })
            }))
        });
        v_flex()
            .w_full()
            .gap_2()
            .p_3()
            .rounded(style::radius(cx))
            .bg(style::white(style::RAISED, cx))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("folder-browser-parent")
                            .outline()
                            .xsmall()
                            .label(t("parent_folder"))
                            .disabled(at_root)
                            .on_click(move |event, window, cx| {
                                if let Some(parent) = &on_parent {
                                    parent(event, window, cx);
                                }
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(SharedString::from(path.clone())),
                    ),
            )
            .child(listing)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("folder-browser-use")
                            .primary()
                            .xsmall()
                            .label(t("use_this_folder"))
                            .on_click(move |_, window, cx| {
                                if let Some(take) = &on_use {
                                    take(&path, window, cx);
                                }
                            }),
                    )
                    .child(
                        Button::new("folder-browser-cancel")
                            .outline()
                            .xsmall()
                            .label(t("cancel"))
                            .on_click(move |event, window, cx| {
                                if let Some(cancel) = &on_cancel {
                                    cancel(event, window, cx);
                                }
                            }),
                    ),
            )
    }
}

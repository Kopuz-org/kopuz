//! Dialogs a shared row can ask for but only a feature crate can draw: one typed registry, no per-dialog plumbing.

use std::any::Any;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

use gpui_kit::{App, Global, SharedString, Window};

type Open<A> = Rc<dyn Fn(A, &mut Window, &mut App)>;

#[derive(Default)]
struct Openers(HashMap<&'static str, Box<dyn Any>>);

impl Global for Openers {}

/// A named way to open one dialog with `A`, which a feature crate registers and anyone may open.
pub struct Opener<A: 'static> {
    name: &'static str,
    args: PhantomData<fn(A)>,
}

impl<A: 'static> Opener<A> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            args: PhantomData,
        }
    }

    /// Install what opening does; the crate that draws the dialog is the only caller.
    pub fn register(&self, open: impl Fn(A, &mut Window, &mut App) + 'static, cx: &mut App) {
        if cx.try_global::<Openers>().is_none() {
            cx.set_global(Openers::default());
        }
        let open: Open<A> = Rc::new(open);
        cx.global_mut::<Openers>()
            .0
            .insert(self.name, Box::new(open));
    }

    /// Open it; an opener nobody registered is logged and ignored.
    pub fn open(&self, args: A, window: &mut Window, cx: &mut App) {
        let open = cx
            .try_global::<Openers>()
            .and_then(|openers| openers.0.get(self.name))
            .and_then(|held| held.downcast_ref::<Open<A>>())
            .cloned();
        match open {
            Some(open) => open(args, window, cx),
            None => tracing::warn!(opener = self.name, "no dialog is registered"),
        }
    }
}

/// What a rename prompt does with the new name.
pub type SaveName = Rc<dyn Fn(String, &mut App)>;

/// A rename prompt: its title, the current name, and what to do with the new one.
pub struct Rename {
    pub title: SharedString,
    pub value: SharedString,
    pub save: SaveName,
}

/// A track's tags: its key, and whether the source lets them be written.
pub const TRACK_METADATA: Opener<(String, bool)> = Opener::new("track-metadata");
/// One set of tags written across a whole selection.
pub const BATCH_METADATA: Opener<Vec<String>> = Opener::new("batch-metadata");
pub const RENAME: Opener<Rename> = Opener::new("rename");
/// Move the playlist with this id into a folder.
pub const FOLDER_PICKER: Opener<String> = Opener::new("folder-picker");
/// The sheet `?` opens.
pub const SHORTCUTS: Opener<()> = Opener::new("shortcuts");
/// The "fetch this URL" box.
pub const URL_DOWNLOAD: Opener<()> = Opener::new("url-download");

//! What a command acts on. Pure data, so it sits below both `commands` and `components`.

use std::fmt;
use std::rc::Rc;

use gpui_kit::{App, SharedString};

/// Row-menu entries only the owning page can carry out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrackActions {
    pub remove_from_playlist: bool,
    pub delete: bool,
    pub reorder: bool,
}

/// What a row's menu asks its list to hand back to the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowEdit {
    Reorder { from: usize, to: usize },
    RemoveFromPlaylist(usize),
    Delete(Vec<String>),
}

type Emit = Rc<dyn Fn(RowEdit, &mut App)>;

/// The list row a track target came from, so a list-scoped command can answer back through it.
#[derive(Clone)]
pub struct RowScope {
    pub index: usize,
    pub rows: usize,
    pub actions: TrackActions,
    emit: Emit,
}

impl RowScope {
    pub fn new(
        index: usize,
        rows: usize,
        actions: TrackActions,
        emit: impl Fn(RowEdit, &mut App) + 'static,
    ) -> Self {
        Self {
            index,
            rows,
            actions,
            emit: Rc::new(emit),
        }
    }

    pub fn reorder(&self, to: usize, cx: &mut App) {
        (self.emit)(
            RowEdit::Reorder {
                from: self.index,
                to,
            },
            cx,
        );
    }

    pub fn remove_from_playlist(&self, cx: &mut App) {
        (self.emit)(RowEdit::RemoveFromPlaylist(self.index), cx);
    }

    pub fn delete(&self, keys: Vec<String>, cx: &mut App) {
        (self.emit)(RowEdit::Delete(keys), cx);
    }
}

impl fmt::Debug for RowScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowScope")
            .field("index", &self.index)
            .field("rows", &self.rows)
            .field("actions", &self.actions)
            .finish()
    }
}

impl PartialEq for RowScope {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.rows == other.rows && self.actions == other.actions
    }
}

impl Eq for RowScope {}

/// The thing under the pointer, the keyboard, or the selection, named well enough to talk about.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Target {
    #[default]
    None,
    /// One or more tracks; `label` names the selection for a confirm or a toast.
    Tracks {
        keys: Vec<SharedString>,
        label: SharedString,
        /// Set when the tracks came from a list row, which is what enables the list-scoped verbs.
        row: Option<RowScope>,
    },
    Album {
        id: SharedString,
        title: SharedString,
    },
    /// `name` only labels it; the key is what opens and plays it.
    Artist { key: String, name: SharedString },
    Playlist {
        id: SharedString,
        name: SharedString,
    },
    Folder {
        id: SharedString,
        name: SharedString,
    },
    /// A row of the queue, by position, since a queue may hold one track twice.
    QueueItem { index: u32, key: SharedString },
    Source {
        id: SharedString,
        name: SharedString,
    },
}

/// What kind of thing a target is, for a command to say what it applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    None,
    Tracks,
    Album,
    Artist,
    Playlist,
    Folder,
    QueueItem,
    Source,
}

impl Target {
    pub fn track(key: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Target::Tracks {
            keys: vec![key.into()],
            label: title.into(),
            row: None,
        }
    }

    pub fn tracks(keys: Vec<SharedString>, label: impl Into<SharedString>) -> Self {
        Target::Tracks {
            keys,
            label: label.into(),
            row: None,
        }
    }

    /// Picked tracks named "Title +3" after the first, rather than a count needing a plural per locale.
    pub fn selection(keys: Vec<SharedString>, first: impl Into<SharedString>) -> Self {
        let first = first.into();
        let label = match keys.len() {
            0 | 1 => first,
            many => SharedString::from(format!("{first} +{}", many - 1)),
        };
        Target::Tracks {
            keys,
            label,
            row: None,
        }
    }

    /// Mark these tracks as coming from a list row.
    pub fn with_row(mut self, scope: RowScope) -> Self {
        if let Target::Tracks { row, .. } = &mut self {
            *row = Some(scope);
        }
        self
    }

    /// The list row these tracks came from, if any.
    pub fn row(&self) -> Option<&RowScope> {
        match self {
            Target::Tracks { row, .. } => row.as_ref(),
            _ => None,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Target::None => Kind::None,
            Target::Tracks { .. } => Kind::Tracks,
            Target::Album { .. } => Kind::Album,
            Target::Artist { .. } => Kind::Artist,
            Target::Playlist { .. } => Kind::Playlist,
            Target::Folder { .. } => Kind::Folder,
            Target::QueueItem { .. } => Kind::QueueItem,
            Target::Source { .. } => Kind::Source,
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Target::None)
    }

    /// How many tracks this stands for; anything that is not a track selection counts as one.
    pub fn count(&self) -> usize {
        match self {
            Target::None => 0,
            Target::Tracks { keys, .. } => keys.len(),
            _ => 1,
        }
    }

    /// The track keys a play or queue command should act on, if any.
    pub fn keys(&self) -> &[SharedString] {
        match self {
            Target::Tracks { keys, .. } => keys,
            _ => &[],
        }
    }

    /// What to call this in a menu item, a confirm, or a toast.
    pub fn label(&self) -> SharedString {
        match self {
            Target::None => SharedString::default(),
            Target::Tracks { label, .. } => label.clone(),
            Target::Album { title, .. } => title.clone(),
            Target::Artist { name, .. } => name.clone(),
            Target::Playlist { name, .. } => name.clone(),
            Target::Folder { name, .. } => name.clone(),
            Target::QueueItem { key, .. } => key.clone(),
            Target::Source { name, .. } => name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_track_is_a_selection_of_one() {
        let target = Target::track("t1", "Blue Monday");
        assert_eq!(target.kind(), Kind::Tracks);
        assert_eq!(target.count(), 1);
        assert_eq!(target.label(), "Blue Monday");
        assert_eq!(target.keys(), ["t1"]);
    }

    #[test]
    fn nothing_selected_acts_on_nothing() {
        let target = Target::default();
        assert!(target.is_none());
        assert_eq!(target.count(), 0);
        assert!(target.keys().is_empty());
    }

    #[test]
    fn an_album_stands_for_itself_not_its_tracks() {
        let target = Target::Album {
            id: "a1".into(),
            title: "Power, Corruption & Lies".into(),
        };
        assert_eq!(target.count(), 1);
        assert!(target.keys().is_empty());
    }
}

//! What the panel lists, and where a drag drops a row.

/// A row's identity, so ordering can be reasoned about without a catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub id: String,
    pub folder: bool,
}

/// The pinned ones in the order they were pinned, then the rest as the catalog gave them.
pub(crate) fn arrange(catalog: Vec<Entry>, pinned: &[String]) -> Vec<Entry> {
    let mut rows: Vec<Entry> = pinned
        .iter()
        .filter_map(|id| catalog.iter().find(|entry| &entry.id == id).cloned())
        .collect();
    for entry in catalog {
        if !rows.contains(&entry) {
            rows.push(entry);
        }
    }
    rows
}

/// The pinned list after dragging `id` onto the slot `at`, with every visible row written down.
pub(crate) fn reorder(rows: &[Entry], id: &str, at: usize) -> Vec<String> {
    let mut ids: Vec<String> = rows.iter().map(|entry| entry.id.clone()).collect();
    let Some(from) = ids.iter().position(|held| held == id) else {
        return ids;
    };
    let held = ids.remove(from);
    // The slot was measured before the row left the list, so a later slot has shifted up by one.
    let to = match at > from {
        true => at.saturating_sub(1),
        false => at,
    };
    ids.insert(to.min(ids.len()), held);
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> Entry {
        Entry {
            id: id.into(),
            folder: false,
        }
    }

    fn ids(rows: &[Entry]) -> Vec<&str> {
        rows.iter().map(|entry| entry.id.as_str()).collect()
    }

    #[test]
    fn nothing_pinned_keeps_the_catalog_order() {
        let catalog = vec![entry("a"), entry("b"), entry("c")];
        assert_eq!(ids(&arrange(catalog, &[])), &["a", "b", "c"]);
    }

    #[test]
    fn a_pinned_row_comes_first() {
        let catalog = vec![entry("a"), entry("b"), entry("c")];
        assert_eq!(ids(&arrange(catalog, &["c".into()])), &["c", "a", "b"]);
    }

    #[test]
    fn a_pin_naming_something_that_is_gone_is_ignored() {
        let catalog = vec![entry("a")];
        assert_eq!(ids(&arrange(catalog, &["gone".into()])), &["a"]);
    }

    #[test]
    fn a_new_playlist_shows_up_after_the_pinned_ones() {
        let catalog = vec![entry("a"), entry("new")];
        assert_eq!(ids(&arrange(catalog, &["a".into()])), &["a", "new"]);
    }

    #[test]
    fn dragging_a_row_down_lands_it_where_it_was_dropped() {
        let rows = vec![entry("a"), entry("b"), entry("c")];
        assert_eq!(reorder(&rows, "a", 2), &["b", "a", "c"]);
        assert_eq!(reorder(&rows, "a", 3), &["b", "c", "a"]);
    }

    #[test]
    fn dragging_a_row_up_lands_it_where_it_was_dropped() {
        let rows = vec![entry("a"), entry("b"), entry("c")];
        assert_eq!(reorder(&rows, "c", 0), &["c", "a", "b"]);
        assert_eq!(reorder(&rows, "c", 1), &["a", "c", "b"]);
    }

    #[test]
    fn dropping_a_row_where_it_already_is_changes_nothing() {
        let rows = vec![entry("a"), entry("b"), entry("c")];
        assert_eq!(reorder(&rows, "b", 1), &["a", "b", "c"]);
        assert_eq!(reorder(&rows, "b", 2), &["a", "b", "c"]);
    }

    #[test]
    fn a_reorder_writes_every_visible_row_down_so_the_order_sticks() {
        let rows = vec![entry("a"), entry("b"), entry("c")];
        assert_eq!(reorder(&rows, "c", 0).len(), rows.len());
    }

    #[test]
    fn dragging_a_row_that_is_not_listed_leaves_the_order_alone() {
        let rows = vec![entry("a"), entry("b")];
        assert_eq!(reorder(&rows, "gone", 0), &["a", "b"]);
    }
}

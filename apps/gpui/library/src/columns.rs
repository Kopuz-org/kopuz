//! The tracks table's saved layout: which columns are shown, how wide, and in what order.

use components::columns::{SPECS, spec};
use store::layout::Column;

/// The columns that are drawn when nobody has changed anything.
pub const SHIPPED: &[&str] = &["title", "artist", "album", "duration"];

/// The saved layout reconciled with this build's columns: unknown ids dropped, new ones hidden, title forced on.
pub fn resolve(saved: &[Column]) -> Vec<Column> {
    if saved.is_empty() {
        return SPECS
            .iter()
            .map(|spec| Column {
                id: spec.id.to_string(),
                visible: SHIPPED.contains(&spec.id),
                width: None,
            })
            .collect();
    }
    let mut columns: Vec<Column> = saved
        .iter()
        .filter(|column| spec(&column.id).is_some())
        .cloned()
        .collect();
    for known in SPECS {
        if !columns.iter().any(|column| column.id == known.id) {
            columns.push(Column {
                id: known.id.to_string(),
                visible: false,
                width: None,
            });
        }
    }
    for column in &mut columns {
        if spec(&column.id).is_some_and(|spec| spec.required) {
            column.visible = true;
        }
    }
    columns
}

/// Put `moved` where `before` sits; dropping a column on itself changes nothing.
pub fn reorder(columns: &mut Vec<Column>, moved: &str, before: &str) {
    if moved == before {
        return;
    }
    let Some(from) = columns.iter().position(|column| column.id == moved) else {
        return;
    };
    let column = columns.remove(from);
    let to = columns
        .iter()
        .position(|held| held.id == before)
        .unwrap_or(columns.len());
    columns.insert(to, column);
}

/// What a resize is clamped to, so a column can never be dragged to nothing or past the window.
pub const MIN_WIDTH: f32 = 60.;
pub const MAX_WIDTH: f32 = 600.;

#[cfg(test)]
mod tests {
    use super::*;
    use components::columns::visible;

    #[test]
    fn an_empty_layout_ships_the_designed_columns() {
        let columns = resolve(&[]);
        let shown: Vec<&str> = visible(&columns)
            .iter()
            .map(|shown| shown.spec.id)
            .collect();
        assert_eq!(shown, SHIPPED);
    }

    #[test]
    fn a_column_from_a_later_version_shows_up_hidden_rather_than_vanishing_the_layout() {
        let saved = vec![Column::new("title"), Column::new("artist")];
        let columns = resolve(&saved);
        assert_eq!(columns.len(), SPECS.len());
        assert!(
            columns
                .iter()
                .find(|column| column.id == "duration")
                .is_some_and(|column| !column.visible)
        );
    }

    #[test]
    fn a_column_this_build_no_longer_has_is_dropped() {
        let saved = vec![Column::new("title"), Column::new("gone")];
        let columns = resolve(&saved);
        assert!(!columns.iter().any(|column| column.id == "gone"));
    }

    #[test]
    fn the_title_column_cannot_be_hidden() {
        let saved = vec![Column {
            id: "title".into(),
            visible: false,
            width: None,
        }];
        let columns = resolve(&saved);
        assert!(columns[0].visible);
    }

    #[test]
    fn a_column_dropped_on_another_lands_in_its_place() {
        let mut columns = resolve(&[]);
        reorder(&mut columns, "duration", "artist");
        let order: Vec<&str> = columns.iter().map(|column| column.id.as_str()).collect();
        assert_eq!(&order[..3], &["title", "duration", "artist"]);
    }

    #[test]
    fn a_column_dropped_on_itself_changes_nothing() {
        let mut columns = resolve(&[]);
        let before = columns.clone();
        reorder(&mut columns, "title", "title");
        assert_eq!(columns, before);
    }
}

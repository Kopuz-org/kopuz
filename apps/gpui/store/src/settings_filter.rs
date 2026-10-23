use std::cell::RefCell;

use gpui_kit::{App, Context, Entity, SharedString};

/// What the Settings search box is filtering by, the rows it has seen, and the row the palette marked.
pub struct SettingsFilter {
    query: Option<SharedString>,
    seen: RefCell<Vec<SharedString>>,
    highlight: Option<SharedString>,
}

impl SettingsFilter {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(_: &mut Context<Self>) -> Self {
        Self {
            query: None,
            seen: RefCell::new(Vec::new()),
            highlight: None,
        }
    }

    /// Filter every settings row by `query`; an empty one puts them all back.
    pub fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        let trimmed = query.trim().to_lowercase();
        let query = (!trimmed.is_empty()).then(|| SharedString::from(trimmed));
        if query.is_some() {
            self.highlight = None;
        } else {
            self.seen.borrow_mut().clear();
        }
        if self.query != query {
            self.query = query;
            crate::notify(cx);
        }
    }

    /// What is being filtered by, lowercased, or nothing when the box is empty.
    pub fn query(&self) -> Option<&SharedString> {
        self.query.as_ref()
    }

    /// Whether a row carrying this text survives the filter. Everything survives an empty box.
    pub fn matches(&self, fields: &[&str]) -> bool {
        let Some(query) = self.query.as_deref() else {
            return true;
        };
        fields.iter().any(|field| hit(field, query))
    }

    /// Remember a searchable line, so "nothing matched" can tell itself from "nothing drawn yet".
    pub fn note(&self, text: &SharedString) {
        if self.query.is_none() {
            return;
        }
        let mut seen = self.seen.borrow_mut();
        if !seen.iter().any(|held| held == text) {
            seen.push(text.clone());
        }
    }

    /// True only once a filtered pass has drawn: before that, an empty page means "not drawn yet".
    pub fn nothing_matched(&self) -> bool {
        let Some(query) = self.query.as_deref() else {
            return false;
        };
        let seen = self.seen.borrow();
        !seen.is_empty() && !seen.iter().any(|held| hit(held, query))
    }

    /// Mark one row by its title; the next jump, or typing in the filter, replaces it.
    pub fn set_highlight(&mut self, title: Option<SharedString>, cx: &mut Context<Self>) {
        if self.highlight != title {
            self.highlight = title;
            crate::notify(cx);
        }
    }

    pub fn highlight(&self) -> Option<&SharedString> {
        self.highlight.as_ref()
    }
}

fn hit(field: &str, query: &str) -> bool {
    field.to_lowercase().contains(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filtering(query: &str) -> SettingsFilter {
        SettingsFilter {
            query: Some(SharedString::from(query.to_string())),
            seen: RefCell::new(Vec::new()),
            highlight: None,
        }
    }

    #[test]
    fn an_empty_box_keeps_every_row() {
        let filter = SettingsFilter {
            query: None,
            seen: RefCell::new(Vec::new()),
            highlight: None,
        };
        assert!(filter.matches(&["Anything"]));
        assert!(!filter.nothing_matched());
    }

    #[test]
    fn a_row_survives_when_any_of_its_fields_hits() {
        let filter = filtering("blur");
        assert!(filter.matches(&["Cover art", "Background BLUR strength"]));
        assert!(!filter.matches(&["Language"]));
    }

    #[test]
    fn nothing_matched_waits_for_a_pass_to_draw() {
        let filter = filtering("zz");
        assert!(!filter.nothing_matched());
        filter.note(&SharedString::from("Language"));
        assert!(filter.nothing_matched());
        filter.note(&SharedString::from("Jazzy"));
        assert!(!filter.nothing_matched());
    }
}

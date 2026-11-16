//! Text field data model with cursor position and selection support.
//!
//! `TextField` wraps a `String` with cursor and selection state, providing
//! methods for inserting/deleting at the cursor, moving the cursor, and
//! positioning the cursor from a tap coordinate.

/// A text field with cursor and optional selection state.
///
/// All byte offsets (`cursor`, `selection`) are kept on character boundaries
/// via `snap_to_char_boundary()`.
#[derive(Debug, Clone)]
pub struct TextField {
    /// The text content.
    pub text: String,
    /// Byte offset of the cursor (always on a char boundary).
    pub cursor: usize,
    /// Optional selection as `(anchor, cursor)` byte offsets.
    /// `anchor` is where the selection started, `cursor` is where it ends.
    pub selection: Option<(usize, usize)>,
}

impl TextField {
    /// Create a new text field with the cursor at the end.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self {
            text,
            cursor,
            selection: None,
        }
    }

    /// Insert text at the cursor position. If there is a selection, it is
    /// replaced by the inserted text.
    pub fn insert_at_cursor(&mut self, s: &str) {
        if let Some((min, max)) = self.normalized_selection() {
            // Replace selection
            self.text.replace_range(min..max, s);
            self.cursor = min + s.len();
            self.selection = None;
        } else {
            self.text.insert_str(self.cursor, s);
            self.cursor += s.len();
        }
        self.cursor = self.snap_to_char_boundary(self.cursor);
    }

    /// Delete at the cursor: removes selection if any, otherwise one character
    /// before the cursor (backspace behavior).
    pub fn delete_at_cursor(&mut self) {
        if let Some((min, max)) = self.normalized_selection() {
            self.text.replace_range(min..max, "");
            self.cursor = min;
            self.selection = None;
        } else if self.cursor > 0 {
            let prev = self.prev_char_boundary(self.cursor);
            self.text.replace_range(prev..self.cursor, "");
            self.cursor = prev;
        }
    }

    /// Move cursor one character to the left, clearing any selection.
    pub fn move_cursor_left(&mut self) {
        self.selection = None;
        if self.cursor > 0 {
            self.cursor = self.prev_char_boundary(self.cursor);
        }
    }

    /// Move cursor one character to the right, clearing any selection.
    pub fn move_cursor_right(&mut self) {
        self.selection = None;
        if self.cursor < self.text.len() {
            self.cursor = self.next_char_boundary(self.cursor);
        }
    }

    /// Move cursor to the start of the text.
    pub fn move_cursor_to_start(&mut self) {
        self.selection = None;
        self.cursor = 0;
    }

    /// Move cursor to the end of the text.
    pub fn move_cursor_to_end(&mut self) {
        self.selection = None;
        self.cursor = self.text.len();
    }

    /// Select all text. Cursor moves to end, anchor at start.
    pub fn select_all(&mut self) {
        self.selection = Some((0, self.text.len()));
        self.cursor = self.text.len();
    }

    /// Approximate the cursor position from a tap's X coordinate.
    ///
    /// `x` is the tap position relative to the input field's left edge.
    /// `avg_char_width` is the approximate width of one character in pixels.
    pub fn set_cursor_from_x(&mut self, x: f32, text_start_x: f32, avg_char_width: f32) {
        self.selection = None;
        if avg_char_width <= 0.0 {
            return;
        }
        let relative_x = (x - text_start_x).max(0.0);
        let char_index = (relative_x / avg_char_width).round() as usize;

        // Convert char index to byte offset
        let byte_offset = self
            .text
            .char_indices()
            .nth(char_index)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());

        self.cursor = byte_offset.min(self.text.len());
    }

    /// Return the normalized selection as `(min, max)` byte offsets,
    /// or `None` if there is no selection.
    pub fn normalized_selection(&self) -> Option<(usize, usize)> {
        self.selection.map(|(a, b)| {
            let min = a.min(b);
            let max = a.max(b);
            (min.min(self.text.len()), max.min(self.text.len()))
        })
    }

    /// Return the text before the cursor.
    pub fn text_before_cursor(&self) -> &str {
        &self.text[..self.cursor.min(self.text.len())]
    }

    /// Return the text after the cursor.
    pub fn text_after_cursor(&self) -> &str {
        &self.text[self.cursor.min(self.text.len())..]
    }

    // ── Internal helpers ────────────────────────────────────────────────

    /// Snap a byte offset to the nearest valid char boundary (rounding down).
    fn snap_to_char_boundary(&self, offset: usize) -> usize {
        let offset = offset.min(self.text.len());
        let mut i = offset;
        while i > 0 && !self.text.is_char_boundary(i) {
            i -= 1;
        }
        i
    }

    /// Find the byte offset of the previous character boundary.
    fn prev_char_boundary(&self, offset: usize) -> usize {
        let mut i = offset.saturating_sub(1);
        while i > 0 && !self.text.is_char_boundary(i) {
            i -= 1;
        }
        i
    }

    /// Find the byte offset of the next character boundary.
    fn next_char_boundary(&self, offset: usize) -> usize {
        let mut i = offset + 1;
        while i < self.text.len() && !self.text.is_char_boundary(i) {
            i += 1;
        }
        i.min(self.text.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_places_cursor_at_end() {
        let f = TextField::new("hello");
        assert_eq!(f.cursor, 5);
        assert_eq!(f.selection, None);
    }

    #[test]
    fn insert_at_end() {
        let mut f = TextField::new("hello");
        f.insert_at_cursor(" world");
        assert_eq!(f.text, "hello world");
        assert_eq!(f.cursor, 11);
    }

    #[test]
    fn insert_at_middle() {
        let mut f = TextField::new("helo");
        f.cursor = 2; // between 'e' and 'l'
        f.insert_at_cursor("l");
        assert_eq!(f.text, "hello");
        assert_eq!(f.cursor, 3);
    }

    #[test]
    fn backspace_at_middle() {
        let mut f = TextField::new("hello");
        f.cursor = 3; // after 'l'
        f.delete_at_cursor();
        assert_eq!(f.text, "helo");
        assert_eq!(f.cursor, 2);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut f = TextField::new("hello");
        f.cursor = 0;
        f.delete_at_cursor();
        assert_eq!(f.text, "hello");
        assert_eq!(f.cursor, 0);
    }

    #[test]
    fn delete_selection() {
        let mut f = TextField::new("hello world");
        f.selection = Some((5, 11)); // " world"
        f.delete_at_cursor();
        assert_eq!(f.text, "hello");
        assert_eq!(f.cursor, 5);
        assert_eq!(f.selection, None);
    }

    #[test]
    fn select_all_then_delete() {
        let mut f = TextField::new("hello");
        f.select_all();
        assert_eq!(f.selection, Some((0, 5)));
        f.delete_at_cursor();
        assert_eq!(f.text, "");
        assert_eq!(f.cursor, 0);
    }

    #[test]
    fn insert_replaces_selection() {
        let mut f = TextField::new("hello world");
        f.selection = Some((6, 11)); // "world"
        f.insert_at_cursor("rust");
        assert_eq!(f.text, "hello rust");
        assert_eq!(f.cursor, 10);
        assert_eq!(f.selection, None);
    }

    #[test]
    fn cursor_movement_clamping() {
        let mut f = TextField::new("hi");
        f.cursor = 0;
        f.move_cursor_left(); // should stay at 0
        assert_eq!(f.cursor, 0);

        f.cursor = 2;
        f.move_cursor_right(); // should stay at 2
        assert_eq!(f.cursor, 2);
    }

    #[test]
    fn cursor_left_right() {
        let mut f = TextField::new("abc");
        f.cursor = 1;
        f.move_cursor_right();
        assert_eq!(f.cursor, 2);
        f.move_cursor_left();
        assert_eq!(f.cursor, 1);
    }

    #[test]
    fn home_and_end() {
        let mut f = TextField::new("hello");
        f.cursor = 3;
        f.move_cursor_to_start();
        assert_eq!(f.cursor, 0);
        f.move_cursor_to_end();
        assert_eq!(f.cursor, 5);
    }

    #[test]
    fn utf8_boundary_safety() {
        // "cafe\u{0301}" = "café" — the accent is a combining character (2 bytes)
        let mut f = TextField::new("caf\u{00e9}!"); // é is 2 bytes
        assert_eq!(f.text.len(), 6); // c(1) a(1) f(1) é(2) !(1)
        f.cursor = 4; // after é
        f.move_cursor_left();
        assert_eq!(f.cursor, 3); // before é
        f.move_cursor_right();
        assert_eq!(f.cursor, 5); // after é
        f.delete_at_cursor(); // delete é
        assert_eq!(f.text, "caf!");
    }

    #[test]
    fn tap_to_position() {
        let mut f = TextField::new("hello");
        // Assume avg_char_width = 10.0, text_start_x = 0.0
        f.set_cursor_from_x(25.0, 0.0, 10.0); // ~2.5 chars → rounds to 3
        assert_eq!(f.cursor, 3);

        f.set_cursor_from_x(0.0, 0.0, 10.0);
        assert_eq!(f.cursor, 0);

        f.set_cursor_from_x(100.0, 0.0, 10.0); // beyond end
        assert_eq!(f.cursor, 5);
    }

    #[test]
    fn normalized_selection_orders_correctly() {
        let mut f = TextField::new("hello");
        f.selection = Some((4, 1)); // backwards selection
        let (min, max) = f.normalized_selection().unwrap();
        assert_eq!(min, 1);
        assert_eq!(max, 4);
    }

    #[test]
    fn text_slices() {
        let f = TextField {
            text: "hello world".to_string(),
            cursor: 5,
            selection: None,
        };
        assert_eq!(f.text_before_cursor(), "hello");
        assert_eq!(f.text_after_cursor(), " world");
    }
}

//! Field lists: how the daemon describes a settings surface a client renders
//! without knowing what is behind it.
//!
//! A source's sign-in form, an integration's credentials and the downloader's
//! options are all the same shape -- a list of [`FieldSpec`] going out, a list
//! of [`FieldValue`] coming back. What a YouTube sign-in or a Jellyfin login
//! actually needs is the daemon's business, so adding a service adds no
//! branch to any frontend.

/// Text a client shows. A `Key` is looked up in its own translations; a
/// `Literal` is a name that is the same everywhere, like a brand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Text {
    Key(String),
    Literal(String),
}

impl Default for Text {
    fn default() -> Self {
        Self::Literal(String::new())
    }
}

impl Text {
    pub fn key(key: impl Into<String>) -> Self {
        Self::Key(key.into())
    }

    pub fn literal(text: impl Into<String>) -> Self {
        Self::Literal(text.into())
    }
}

/// A glyph. `Class` is an icon-font class the app already ships; `Svg` is a
/// path, for the services whose logo no icon font has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Icon {
    Class(String),
    Svg(String),
}

impl Default for Icon {
    fn default() -> Self {
        Self::Class(String::new())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChoiceOption {
    pub value: String,
    pub label: Text,
}

/// What control a field is edited with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FieldKind {
    #[default]
    Text,
    /// Never sent back out with a value, and an empty one on the way in means
    /// "leave it alone" -- clearing a credential is its own call.
    Secret,
    Url,
    Toggle,
    /// A single filesystem path, with whatever picker the platform has.
    Directory,
    /// `custom` lets a value outside the list be typed in.
    Choice {
        options: Vec<ChoiceOption>,
        custom: bool,
    },
    Radio {
        options: Vec<ChoiceOption>,
    },
    /// No control: a line of explanation, which `show_when` can hide.
    Note,
}

/// One row of a published settings surface.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldSpec {
    pub key: String,
    pub label: Text,
    /// Rendered under the control.
    pub help: Option<Text>,
    pub placeholder: Option<Text>,
    /// Starts a titled group before this row.
    pub section: Option<Text>,
    pub kind: FieldKind,
    pub required: bool,
    /// The current value, or the default for a new one. Never set for a
    /// [`FieldKind::Secret`].
    pub value: Option<String>,
    /// The settings key behind this row, where there is one, so a client can
    /// render it locked when a managed settings file pins it.
    pub config_key: Option<String>,
    /// Show this row only while another field holds a given value.
    pub show_when: Option<FieldValue>,
}

/// One answer. A [`FieldKind::Toggle`] is `"true"` or `"false"`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldValue {
    pub key: String,
    pub value: String,
}

impl FieldValue {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    pub fn is_on(&self) -> bool {
        self.value == "true"
    }
}

/// Read the current value of one row of a published list.
pub fn spec_value<'a>(fields: &'a [FieldSpec], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| field.value.as_deref())
}

/// Read one value out of an answer list.
pub fn value_of<'a>(values: &'a [FieldValue], key: &str) -> Option<&'a str> {
    values
        .iter()
        .find(|value| value.key == key)
        .map(|value| value.value.as_str())
}

/// Read a toggle, falling back to what the daemon already had.
pub fn toggle_of(values: &[FieldValue], key: &str, current: bool) -> bool {
    value_of(values, key).map_or(current, |value| value == "true")
}

/// Why a draft cannot be saved. `field` names the row it belongs under; a
/// problem with none is about the form as a whole.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Problem {
    pub field: Option<String>,
    pub label: Text,
}

impl Problem {
    pub fn on(field: impl Into<String>, label: Text) -> Self {
        Self {
            field: Some(field.into()),
            label,
        }
    }
}

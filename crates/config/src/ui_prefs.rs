//! What a frontend changes by being used: sorts, view modes and layout, kept per frontend in the daemon's preference store.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    AlbumSortField, AlbumViewMode, ArtistSortField, ArtistViewOrder, HOME_SECTION_KEYS,
    HomeSection, ListenNowStyle, SortCriterion, SortDirection, SortOrder, TrackSortField,
    default_home_sections, default_sidebar_order,
};

/// Stored one key per field, named as the field and holding it as JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefs {
    pub source_explicitly_set: bool,
    /// Fullscreen: the Up Next / Lyrics side panel is collapsed.
    pub fullscreen_tabs_collapsed: bool,
    pub sort_order: SortOrder,
    pub album_sort: Vec<SortCriterion<AlbumSortField>>,
    pub library_sort: Vec<SortCriterion<TrackSortField>>,
    pub artist_album_sort: Vec<SortCriterion<AlbumSortField>>,
    pub artist_sort: Vec<SortCriterion<ArtistSortField>>,
    pub album_view_mode: AlbumViewMode,
    pub artist_album_view_mode: AlbumViewMode,
    pub artists_view_mode: AlbumViewMode,
    pub artist_view_order: ArtistViewOrder,
    pub sidebar_order: Vec<String>,
    pub home_sections: Vec<HomeSection>,
    pub listen_now_style: ListenNowStyle,
    pub hero_height: u32,
}

impl Default for UiPrefs {
    fn default() -> Self {
        let album_sort = vec![SortCriterion::new(
            AlbumSortField::Title,
            SortDirection::Asc,
        )];
        Self {
            source_explicitly_set: false,
            fullscreen_tabs_collapsed: false,
            sort_order: SortOrder::Title,
            artist_album_sort: album_sort.clone(),
            album_sort,
            library_sort: vec![SortCriterion::new(
                TrackSortField::Title,
                SortDirection::Asc,
            )],
            artist_sort: vec![SortCriterion::new(
                ArtistSortField::Name,
                SortDirection::Asc,
            )],
            album_view_mode: AlbumViewMode::Grid,
            artist_album_view_mode: AlbumViewMode::Grid,
            artists_view_mode: AlbumViewMode::Grid,
            artist_view_order: ArtistViewOrder::Tracks,
            sidebar_order: default_sidebar_order(),
            home_sections: default_home_sections(),
            listen_now_style: ListenNowStyle::List,
            hero_height: 300,
        }
    }
}

impl UiPrefs {
    /// Each field under its key, as the JSON value a preference stores; compare these, not their text, whose key order varies.
    pub fn entries(&self) -> BTreeMap<String, serde_json::Value> {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(fields)) => fields.into_iter().collect(),
            _ => BTreeMap::new(),
        }
    }

    /// The defaults with every stored entry laid over them, normalised; a value that no longer reads drops only itself.
    pub fn from_entries<'a>(stored: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        Self::from_values(stored.into_iter().filter_map(|(key, text)| {
            match serde_json::from_str::<serde_json::Value>(text) {
                Ok(value) => Some((key, value)),
                Err(_) => {
                    tracing::warn!(key, "stored UI preference is not JSON");
                    None
                }
            }
        }))
    }

    /// Like [`Self::from_entries`], over values already parsed.
    pub fn from_values<'a>(stored: impl IntoIterator<Item = (&'a str, serde_json::Value)>) -> Self {
        let defaults = Self::default();
        let Ok(serde_json::Value::Object(mut object)) = serde_json::to_value(&defaults) else {
            return defaults;
        };
        for (key, value) in stored {
            if !object.contains_key(key) {
                continue;
            }
            let previous = object.insert(key.to_string(), value);
            if serde_json::from_value::<Self>(serde_json::Value::Object(object.clone())).is_err() {
                tracing::warn!(key, "stored UI preference ignored");
                if let Some(previous) = previous {
                    object.insert(key.to_string(), previous);
                }
            }
        }
        let mut prefs: Self =
            serde_json::from_value(serde_json::Value::Object(object)).unwrap_or(defaults);
        prefs.normalize();
        prefs
    }

    /// Keep every known home section and sidebar entry exactly once, appending what a stored order lacks.
    pub fn normalize(&mut self) {
        let allowed: HashSet<&str> = HOME_SECTION_KEYS.iter().copied().collect();
        let mut seen: HashSet<String> = HashSet::new();
        self.home_sections.retain(|section| {
            allowed.contains(section.key.as_str()) && seen.insert(section.key.clone())
        });
        for key in HOME_SECTION_KEYS {
            if !seen.contains(*key) {
                self.home_sections.push(HomeSection {
                    key: (*key).to_string(),
                    enabled: true,
                });
            }
        }

        let all_keys = default_sidebar_order();
        for key in &all_keys {
            if !self.sidebar_order.iter().any(|k| k == key) {
                self.sidebar_order.push(key.to_string());
            }
        }
        self.sidebar_order.retain(|k| all_keys.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_read_back_as_what_was_stored() {
        let prefs = UiPrefs {
            album_sort: vec![SortCriterion::new(
                AlbumSortField::Year,
                SortDirection::Desc,
            )],
            hero_height: 420,
            listen_now_style: ListenNowStyle::Cards,
            ..UiPrefs::default()
        };
        let entries = prefs.entries();
        assert_eq!(
            entries.get("album_sort"),
            Some(&serde_json::json!([{"field": "Year", "direction": "Desc"}]))
        );
        assert_eq!(entries.get("hero_height"), Some(&serde_json::json!(420)));
        let texts: Vec<(String, String)> = entries
            .iter()
            .map(|(key, value)| (key.clone(), value.to_string()))
            .collect();
        let back = UiPrefs::from_entries(texts.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        assert_eq!(back, prefs);
    }

    #[test]
    fn a_stored_value_reads_whatever_order_its_keys_are_in() {
        let prefs =
            UiPrefs::from_entries([("album_sort", r#"[{"direction":"Desc","field":"Year"}]"#)]);
        assert_eq!(
            prefs.entries().get("album_sort"),
            UiPrefs::from_entries([("album_sort", r#"[{"field":"Year","direction":"Desc"}]"#)])
                .entries()
                .get("album_sort")
        );
    }

    #[test]
    fn an_unreadable_value_drops_only_itself() {
        let prefs = UiPrefs::from_entries([
            ("hero_height", "\"tall\""),
            ("album_view_mode", "\"List\""),
            ("sort_order", "not json"),
            ("unknown", "1"),
        ]);
        assert_eq!(prefs.hero_height, UiPrefs::default().hero_height);
        assert_eq!(prefs.album_view_mode, AlbumViewMode::List);
        assert_eq!(prefs.sort_order, SortOrder::Title);
    }

    #[test]
    fn a_stored_order_keeps_its_place_and_gains_what_it_lacks() {
        let prefs = UiPrefs::from_entries([
            ("sidebar_order", r#"["albums","gone","home"]"#),
            (
                "home_sections",
                r#"[{"key":"playlists","enabled":false},{"key":"nope","enabled":true}]"#,
            ),
        ]);
        assert_eq!(&prefs.sidebar_order[..2], ["albums", "home"]);
        assert_eq!(prefs.sidebar_order.len(), default_sidebar_order().len());
        assert_eq!(prefs.home_sections[0].key, "playlists");
        assert!(!prefs.home_sections[0].enabled);
        assert_eq!(prefs.home_sections.len(), HOME_SECTION_KEYS.len());
    }
}

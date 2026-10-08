//! `search` and `music/get_search_suggestions` answers: the top result card,
//! the result shelves, a "did you mean", and the completions under the box.

use serde_json::Value;

use super::items::item;
use super::shelves::sections;
use super::{renderer, text};
use crate::ytmusic::discover::{DiscoverItem, DiscoverShelf, ShelfLayout};

/// One search filter: the id a client passes back, the translation key of
/// its label, the `params` YouTube takes for it, and the title of the
/// results shelf it matches.
#[derive(Debug)]
pub struct Filter {
    pub id: &'static str,
    pub label: &'static str,
    params: &'static str,
    title: &'static str,
    layout: ShelfLayout,
}

/// The filter that answers with everything: the top result, then a shelf
/// per kind.
pub const ALL: &str = "all";
pub const ALL_LABEL: &str = "search_filter_all";

/// In the web app's order, which is also the order of the All results.
pub const FILTERS: [Filter; 9] = [
    Filter {
        id: "songs",
        label: "search_filter_songs",
        params: "EgWKAQIIAWoMEA4QChADEAQQCRAF",
        title: "Songs",
        layout: ShelfLayout::List,
    },
    Filter {
        id: "videos",
        label: "search_filter_videos",
        params: "EgWKAQIQAWoMEA4QChADEAQQCRAF",
        title: "Videos",
        layout: ShelfLayout::List,
    },
    Filter {
        id: "albums",
        label: "albums",
        params: "EgWKAQIYAWoMEA4QChADEAQQCRAF",
        title: "Albums",
        layout: ShelfLayout::Carousel,
    },
    Filter {
        id: "artists",
        label: "artists",
        params: "EgWKAQIgAWoMEA4QChADEAQQCRAF",
        title: "Artists",
        layout: ShelfLayout::Carousel,
    },
    Filter {
        id: "community_playlists",
        label: "search_filter_community_playlists",
        params: "EgeKAQQoAEABagwQDhAKEAMQBBAJEAU=",
        title: "Community playlists",
        layout: ShelfLayout::Carousel,
    },
    Filter {
        id: "featured_playlists",
        label: "search_filter_featured_playlists",
        params: "EgeKAQQoADgBagwQDhAKEAMQBBAJEAU=",
        title: "Featured playlists",
        layout: ShelfLayout::Carousel,
    },
    Filter {
        id: "episodes",
        label: "search_filter_episodes",
        params: "EgWKAQJIAWoMEA4QChADEAQQCRAF",
        title: "Episodes",
        layout: ShelfLayout::List,
    },
    Filter {
        id: "profiles",
        label: "search_filter_profiles",
        params: "EgWKAQJYAWoMEA4QChADEAQQCRAF",
        title: "Profiles",
        layout: ShelfLayout::Carousel,
    },
    Filter {
        id: "podcasts",
        label: "catalog_page_podcasts",
        params: "EgWKAQJQAWoMEA4QChADEAQQCRAF",
        title: "Podcasts",
        layout: ShelfLayout::Carousel,
    },
];

/// The account's own library, which only a session has.
pub const LIBRARY: Filter = Filter {
    id: "library",
    label: "library",
    params: "agIYBA==",
    title: "Library",
    layout: ShelfLayout::List,
};

pub fn filter(id: &str) -> Option<&'static Filter> {
    FILTERS
        .iter()
        .chain([&LIBRARY])
        .find(|filter| filter.id == id)
}

impl Filter {
    pub fn params(&self) -> &'static str {
        self.params
    }
}

/// What a search found. A filtered search is one long shelf, and
/// `continuation` is more of it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchPage {
    pub shelves: Vec<DiscoverShelf>,
    pub continuation: Option<String>,
    /// "Did you mean" or "Showing results for".
    pub correction: Option<String>,
}

pub(crate) fn parse_search(filter: Option<&Filter>, response: &Value) -> SearchPage {
    let contents = &response["contents"];
    let list = match contents["tabbedSearchResultsRenderer"]["tabs"].as_array() {
        Some(tabs) => {
            let Some(tab) = tabs
                .iter()
                .find(|t| t["tabRenderer"]["selected"].as_bool() == Some(true))
                .or(tabs.first())
            else {
                return SearchPage::default();
            };
            &tab["tabRenderer"]["content"]["sectionListRenderer"]
        }
        None => &contents["sectionListRenderer"],
    };
    let found = sections(&list["contents"]);
    let correction = correction(&list["contents"]);
    let Some(filter) = filter else {
        return SearchPage {
            shelves: by_category(found),
            continuation: None,
            correction,
        };
    };
    let mut shelves = found;
    // Some rows under the Videos filter carry no video type and no "Video"
    // label, which would make them songs.
    if filter.id == "videos" {
        for shelf in &mut shelves {
            for item in &mut shelf.items {
                if let DiscoverItem::Song(track) = item {
                    *item = DiscoverItem::Video(track.clone());
                }
            }
        }
    }
    let continuation = shelves
        .iter_mut()
        .rev()
        .find_map(|shelf| shelf.continuation.take());
    SearchPage {
        shelves,
        continuation,
        correction,
    }
}

fn correction(contents: &Value) -> Option<String> {
    contents
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|entry| {
            entry["itemSectionRenderer"]["contents"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .find_map(|inner| match renderer(inner) {
            Some(("showingResultsForRenderer" | "didYouMeanRenderer", r)) => {
                text(&r["correctedQuery"])
            }
            _ => None,
        })
}

/// YouTube sends the All results as the top result card and then one
/// untitled row per result, every kind interleaved. Those rows are sorted
/// into one titled shelf per kind, in YouTube's order within each. A titled
/// shelf, as YouTube used to send them, stays as it is.
fn by_category(found: Vec<DiscoverShelf>) -> Vec<DiscoverShelf> {
    let mut out = Vec::new();
    let mut buckets: Vec<Vec<DiscoverItem>> = FILTERS.iter().map(|_| Vec::new()).collect();
    for mut shelf in found {
        if !shelf.title.is_empty() || shelf.layout != ShelfLayout::List {
            shelf.search_filter = FILTERS
                .iter()
                .find(|filter| filter.title == shelf.title)
                .map(|filter| filter.id);
            out.push(shelf);
            continue;
        }
        for item in shelf.items {
            match category(&item).and_then(|id| FILTERS.iter().position(|f| f.id == id)) {
                Some(n) => buckets[n].push(item),
                None => tracing::debug!(?item, "search result of no filter's kind"),
            }
        }
    }
    out.extend(
        FILTERS
            .iter()
            .zip(buckets)
            .filter(|(_, items)| !items.is_empty())
            .map(|(filter, items)| DiscoverShelf {
                title: filter.title.to_string(),
                strapline: None,
                more: None,
                items,
                layout: filter.layout,
                continuation: None,
                search_filter: Some(filter.id),
            }),
    );
    out
}

/// The filter whose results `item` would be among.
fn category(item: &DiscoverItem) -> Option<&'static str> {
    Some(match item {
        DiscoverItem::Song(_) => "songs",
        DiscoverItem::Video(_) => "videos",
        DiscoverItem::Episode { .. } => "episodes",
        DiscoverItem::Album { .. } => "albums",
        // Profiles are channels too, told apart by their row's label.
        DiscoverItem::Artist { subtitle, .. }
            if subtitle
                .as_deref()
                .is_some_and(|s| s.starts_with("Profile")) =>
        {
            "profiles"
        }
        DiscoverItem::Artist { .. } => "artists",
        // YouTube Music's own playlists, the Featured filter's, are its
        // RDCLAK mixes; everyone else's are community playlists.
        DiscoverItem::Playlist { playlist_id, .. } if playlist_id.starts_with("RDCLAK") => {
            "featured_playlists"
        }
        DiscoverItem::Playlist { .. } => "community_playlists",
        DiscoverItem::Podcast { .. } => "podcasts",
        DiscoverItem::Mood { .. } | DiscoverItem::Page { .. } => return None,
    })
}

/// One completion under the search box: a query to run, or a direct hit.
#[derive(Debug, Clone, PartialEq)]
pub enum Suggestion {
    Query { text: String, from_history: bool },
    Item(DiscoverItem),
}

pub(crate) fn parse_suggestions(response: &Value) -> Vec<Suggestion> {
    response["contents"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|section| {
            section["searchSuggestionsSectionRenderer"]["contents"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .filter_map(|entry| match renderer(entry)? {
            ("searchSuggestionRenderer", r) => query(r, false),
            ("historySuggestionRenderer", r) => query(r, true),
            _ => item(entry).map(Suggestion::Item),
        })
        .collect()
}

fn query(r: &Value, from_history: bool) -> Option<Suggestion> {
    let text = r["navigationEndpoint"]["searchEndpoint"]["query"]
        .as_str()
        .map(str::to_string)
        .or_else(|| text(&r["suggestion"]))?;
    Some(Suggestion::Query { text, from_history })
}

/// A continuation of search results is more of the one filtered shelf.
pub(crate) fn continued(mut page: crate::ytmusic::discover::BrowsePage) -> SearchPage {
    let continuation = page
        .shelves
        .iter_mut()
        .rev()
        .find_map(|shelf| shelf.continuation.take())
        .or(page.continuation);
    SearchPage {
        shelves: page.shelves,
        continuation,
        correction: None,
    }
}

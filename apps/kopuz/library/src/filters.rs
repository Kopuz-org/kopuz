//! What the library is showing, in a form that fits in the route: the facet and every active chip.

use api::{AlbumQuery, ArtistQuery, TrackFilter, TrackSort};
use config::{SortCriterion, SortDirection, TrackSortField};
pub use store::layout::{album_sort_field, artist_sort_field};

use crate::facet::Facet;

/// The facet on screen and every chip that narrows it. Encoded into the route, so back, forward
/// and a restart all land on the same listing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    pub facet: Facet,
    pub search: String,
    pub favorites: bool,
    pub downloaded: bool,
    pub genre: Option<String>,
    pub years: Option<(u16, u16)>,
    pub folder: Option<String>,
}

impl Filters {
    /// Nothing narrowed, on the given facet.
    pub fn on(facet: Facet) -> Self {
        Self {
            facet,
            ..Self::default()
        }
    }

    /// Whether any chip is on; the search box is a chip for this purpose.
    pub fn is_narrowed(&self) -> bool {
        !self.search.trim().is_empty()
            || self.favorites
            || self.downloaded
            || self.genre.is_some()
            || self.years.is_some()
            || self.folder.is_some()
    }

    /// How many chips are lit, for the "clear all" control.
    pub fn chip_count(&self) -> usize {
        usize::from(self.favorites)
            + usize::from(self.downloaded)
            + usize::from(self.genre.is_some())
            + usize::from(self.years.is_some())
            + usize::from(self.folder.is_some())
    }

    /// Drop every chip and the search text, keeping the facet.
    pub fn cleared(&self) -> Self {
        Self::on(self.facet)
    }

    /// The same narrowing on the Favorites page, which only ever lists liked tracks.
    pub fn held_to_liked(mut self) -> Self {
        self.facet = Facet::Tracks;
        self.favorites = true;
        self
    }

    /// The tracks the daemon is asked for: every chip and the order, narrowed and sorted there.
    pub fn track_filter(&self, sort: &Sort) -> TrackFilter {
        TrackFilter {
            search: non_empty(&self.search),
            album: None,
            genre: self.genre.clone(),
            favorite: self.favorites.then_some(true),
            sort: sort.to_track_sort(),
            downloaded: self.downloaded.then_some(true),
            offline: None,
            year_from: self.years.map(|(from, _)| from),
            year_to: self.years.map(|(_, to)| to),
            reverse: sort.reverses(),
        }
    }

    /// The albums the daemon is asked for, filtered and ordered there.
    pub fn album_query(&self, sort: &Sort) -> AlbumQuery {
        AlbumQuery {
            search: non_empty(&self.search),
            genre: self.genre.clone(),
            year_from: self.years.map(|(from, _)| from),
            year_to: self.years.map(|(_, to)| to),
            artist_key: None,
            ids: None,
            downloaded: None,
            offline: None,
            sort: album_sort_field(&sort.field)
                .map(|field| SortCriterion::new(field, sort.direction()))
                .into_iter()
                .collect(),
        }
    }

    /// The artists the daemon is asked for, filtered and ordered there.
    pub fn artist_query(&self, sort: &Sort) -> ArtistQuery {
        ArtistQuery {
            search: non_empty(&self.search),
            downloaded: None,
            offline: None,
            sort: artist_sort_field(&sort.field)
                .map(|field| SortCriterion::new(field, sort.direction()))
                .into_iter()
                .collect(),
        }
    }

    /// Whether this chip can be offered at all on the facet that is showing.
    pub fn applies(&self, chip: Chip) -> bool {
        match chip {
            // A year and a genre belong to an album; nothing else on the wire carries either.
            Chip::Years | Chip::Genre => self.facet == Facet::Albums || self.facet == Facet::Tracks,
            // A folder is a path under a scan root, so only a track has one.
            Chip::Folder | Chip::Favorites | Chip::Downloaded => self.facet == Facet::Tracks,
        }
    }

    /// The route payload: `f=tracks&q=blue&fav=1`, with `&`, `=` and `%` escaped in the values.
    pub fn encode(&self) -> String {
        let mut parts = vec![format!("f={}", self.facet.name())];
        let search = self.search.trim();
        if !search.is_empty() {
            parts.push(format!("q={}", escape(search)));
        }
        if self.favorites {
            parts.push("fav=1".to_string());
        }
        if self.downloaded {
            parts.push("dl=1".to_string());
        }
        if let Some(genre) = &self.genre {
            parts.push(format!("g={}", escape(genre)));
        }
        if let Some((from, to)) = self.years {
            parts.push(format!("y={from}-{to}"));
        }
        if let Some(folder) = &self.folder {
            parts.push(format!("d={}", escape(folder)));
        }
        parts.join("&")
    }

    /// Read back what `encode` wrote; anything unrecognised is ignored rather than refused.
    pub fn decode(encoded: &str) -> Self {
        let mut filters = Self::default();
        for part in encoded.split('&').filter(|part| !part.is_empty()) {
            let (key, value) = match part.split_once('=') {
                Some(pair) => pair,
                None => continue,
            };
            match key {
                "f" => filters.facet = Facet::from_name(value).unwrap_or_default(),
                "q" => filters.search = unescape(value),
                "fav" => filters.favorites = value == "1",
                "dl" => filters.downloaded = value == "1",
                "g" => filters.genre = non_empty(&unescape(value)),
                "y" => filters.years = decode_years(value),
                "d" => filters.folder = non_empty(&unescape(value)),
                _ => {}
            }
        }
        filters
    }
}

/// The chips the filter bar offers, in the order it draws them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chip {
    Favorites,
    Downloaded,
    Genre,
    Years,
    Folder,
}

impl Chip {
    pub const ALL: &'static [Chip] = &[
        Chip::Favorites,
        Chip::Downloaded,
        Chip::Genre,
        Chip::Years,
        Chip::Folder,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Chip::Favorites => "library-chip-favorites",
            Chip::Downloaded => "library-chip-downloaded",
            Chip::Genre => "library-chip-genre",
            Chip::Years => "library-chip-years",
            Chip::Folder => "library-chip-folder",
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            Chip::Favorites => "favorites",
            Chip::Downloaded => "library_chip_downloaded",
            Chip::Genre => "genre",
            Chip::Years => "library_chip_years",
            Chip::Folder => "library_chip_folder",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Chip::Favorites => components::icons::HEART,
            Chip::Downloaded => components::icons::DOWNLOAD,
            Chip::Genre => components::icons::GRID,
            Chip::Years => components::icons::CLOCK,
            Chip::Folder => components::icons::FOLDER,
        }
    }

    /// Whether the chip opens a menu rather than simply toggling.
    pub fn has_menu(self) -> bool {
        matches!(self, Chip::Genre | Chip::Years | Chip::Folder)
    }
}

/// A facet's order: the column or field it runs on, and which way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sort {
    pub field: String,
    pub descending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            field: "title".into(),
            descending: false,
        }
    }
}

impl Sort {
    pub fn from_layout(sort: &store::layout::Sort) -> Self {
        Self {
            field: sort.field.clone(),
            descending: sort.descending,
        }
    }

    pub fn to_layout(&self) -> store::layout::Sort {
        store::layout::Sort {
            field: self.field.clone(),
            descending: self.descending,
        }
    }

    /// Clicking a header: the same field flips direction, a new one starts ascending.
    pub fn toggled(&self, field: &str) -> Self {
        match self.field == field {
            true => Self {
                field: self.field.clone(),
                descending: !self.descending,
            },
            false => Self {
                field: field.to_string(),
                descending: false,
            },
        }
    }

    fn direction(&self) -> SortDirection {
        match self.descending {
            true => SortDirection::Desc,
            false => SortDirection::Asc,
        }
    }

    /// What the daemon is asked to order by. Play count is its own sort rather than a criterion.
    pub fn to_track_sort(&self) -> TrackSort {
        if self.field == "plays" {
            return TrackSort::PlayCount;
        }
        let Some(field) = track_sort_field(&self.field) else {
            return TrackSort::Default;
        };
        TrackSort::Fields(vec![SortCriterion::new(field, self.direction())])
    }

    /// Play count runs high to low by itself, so ascending is that order reversed.
    pub fn reverses(&self) -> bool {
        self.field == "plays" && !self.descending
    }
}

/// The column ids the daemon can order a track listing by.
pub fn track_sort_field(id: &str) -> Option<TrackSortField> {
    Some(match id {
        "title" => TrackSortField::Title,
        "artist" => TrackSortField::Artist,
        "album" => TrackSortField::Album,
        "duration" => TrackSortField::Duration,
        "added" => TrackSortField::DateAdded,
        _ => return None,
    })
}

/// Whether the daemon can order by this column at all; anything else sorts what is held.
pub fn is_server_sortable(id: &str) -> bool {
    id == "plays" || track_sort_field(id).is_some()
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    match trimmed.is_empty() {
        true => None,
        false => Some(trimmed.to_string()),
    }
}

fn decode_years(value: &str) -> Option<(u16, u16)> {
    let (from, to) = value.split_once('-')?;
    let from = from.parse().ok()?;
    let to = to.parse().ok()?;
    match from <= to {
        true => Some((from, to)),
        false => Some((to, from)),
    }
}

/// The three characters the encoding itself uses; everything else travels as written.
fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '%' => escaped.push_str("%25"),
            '&' => escaped.push_str("%26"),
            '=' => escaped.push_str("%3D"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let code = rest.get(at + 1..at + 3);
        match code {
            Some("25") => out.push('%'),
            Some("26") => out.push('&'),
            Some("3D") | Some("3d") => out.push('='),
            _ => {
                out.push('%');
                rest = &rest[at + 1..];
                continue;
            }
        }
        rest = &rest[at + 3..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{AlbumSortField, ArtistSortField};

    #[test]
    fn a_filter_set_survives_the_route() {
        let filters = Filters {
            facet: Facet::Albums,
            search: "blue monday".into(),
            favorites: true,
            downloaded: false,
            genre: Some("Post-Punk".into()),
            years: Some((1980, 1989)),
            folder: None,
        };
        assert_eq!(Filters::decode(&filters.encode()), filters);
    }

    #[test]
    fn a_value_holding_the_encoding_s_own_characters_comes_back_whole() {
        let filters = Filters {
            facet: Facet::Tracks,
            search: "rock & roll = 100%".into(),
            folder: Some("/music/a=b&c".into()),
            ..Filters::default()
        };
        let back = Filters::decode(&filters.encode());
        assert_eq!(back.search, "rock & roll = 100%");
        assert_eq!(back.folder.as_deref(), Some("/music/a=b&c"));
    }

    #[test]
    fn an_empty_route_is_the_default_view() {
        assert_eq!(Filters::decode(""), Filters::default());
        assert_eq!(Filters::decode("").facet, Facet::Playlists);
    }

    #[test]
    fn a_key_from_a_later_version_is_ignored() {
        let filters = Filters::decode("f=artists&what=3");
        assert_eq!(filters.facet, Facet::Artists);
        assert!(!filters.is_narrowed());
    }

    #[test]
    fn clearing_keeps_the_facet_and_drops_the_chips() {
        let filters = Filters {
            facet: Facet::Artists,
            favorites: true,
            search: "x".into(),
            ..Filters::default()
        };
        let cleared = filters.cleared();
        assert_eq!(cleared.facet, Facet::Artists);
        assert!(!cleared.is_narrowed());
    }

    #[test]
    fn every_track_chip_reaches_the_daemon() {
        let filters = Filters {
            downloaded: true,
            years: Some((1980, 1989)),
            ..Filters::on(Facet::Tracks)
        };
        let filter = filters.track_filter(&Sort::default());
        assert_eq!(filter.downloaded, Some(true));
        assert_eq!((filter.year_from, filter.year_to), (Some(1980), Some(1989)));
    }

    #[test]
    fn the_album_grid_asks_for_its_chips_and_its_order() {
        let filters = Filters {
            search: " blue ".into(),
            genre: Some("Jazz".into()),
            years: Some((1950, 1959)),
            ..Filters::on(Facet::Albums)
        };
        let sort = Sort {
            field: "year".into(),
            descending: true,
        };
        let query = filters.album_query(&sort);
        assert_eq!(query.search.as_deref(), Some("blue"));
        assert_eq!(query.genre.as_deref(), Some("Jazz"));
        assert_eq!((query.year_from, query.year_to), (Some(1950), Some(1959)));
        assert_eq!(
            query.sort,
            [SortCriterion::new(
                AlbumSortField::Year,
                SortDirection::Desc
            )]
        );
    }

    #[test]
    fn the_artist_grid_orders_by_what_it_offers() {
        let sort = Sort {
            field: "albums".into(),
            descending: true,
        };
        let query = Filters::on(Facet::Artists).artist_query(&sort);
        assert_eq!(
            query.sort,
            [SortCriterion::new(
                ArtistSortField::Albums,
                SortDirection::Desc
            )]
        );
        assert_eq!(query.search, None);
    }

    #[test]
    fn ascending_play_count_reverses_the_daemon_order() {
        let sort = Sort {
            field: "plays".into(),
            descending: false,
        };
        assert!(Filters::on(Facet::Tracks).track_filter(&sort).reverse);
    }

    #[test]
    fn a_chip_is_only_offered_where_it_means_something() {
        let tracks = Filters::on(Facet::Tracks);
        assert!(tracks.applies(Chip::Favorites));
        assert!(tracks.applies(Chip::Genre));
        let albums = Filters::on(Facet::Albums);
        assert!(albums.applies(Chip::Years));
        assert!(!albums.applies(Chip::Folder));
        for chip in Chip::ALL {
            assert!(!Filters::on(Facet::Artists).applies(*chip));
            assert!(!Filters::on(Facet::Playlists).applies(*chip));
        }
    }

    #[test]
    fn a_header_click_flips_its_own_column_and_restarts_another() {
        let sort = Sort::default();
        let flipped = sort.toggled("title");
        assert!(flipped.descending);
        let moved = flipped.toggled("artist");
        assert_eq!(moved.field, "artist");
        assert!(!moved.descending);
    }

    #[test]
    fn play_count_is_its_own_sort_not_a_criterion() {
        let sort = Sort {
            field: "plays".into(),
            descending: true,
        };
        assert_eq!(sort.to_track_sort(), TrackSort::PlayCount);
    }

    #[test]
    fn a_column_the_daemon_cannot_order_by_asks_for_nothing() {
        let sort = Sort {
            field: "format".into(),
            descending: false,
        };
        assert_eq!(sort.to_track_sort(), TrackSort::Default);
        assert!(!is_server_sortable("format"));
        assert!(is_server_sortable("added"));
    }

    #[test]
    fn a_backwards_year_range_reads_the_right_way_round() {
        assert_eq!(Filters::decode("y=1999-1990").years, Some((1990, 1999)));
    }
}

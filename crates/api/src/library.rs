use crate::player::TrackKind;

/// Library ordering. `Default` is the daemon's own choice, so a caller that
/// does not care says nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TrackSort {
    #[default]
    Default,
    Title,
    Artist,
    Album,
    DateAdded,
    PlayCount,
    /// Stacked user criteria (the library's sort control): the first field
    /// decides, the rest break ties. Empty means [`TrackSort::Default`].
    Fields(Vec<config::SortCriterion<config::TrackSortField>>),
}

/// A track row on the wire. `key` is the stable library ref used everywhere
/// else in the API; local filesystem paths and credentialed remote URLs never
/// appear here.
///
/// This is what frontends render, so it carries everything a row displays --
/// including `artwork`, which says whether a cover exists at all.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackInfo {
    pub key: String,
    pub uid: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: String,
    pub duration_ms: Option<u64>,
    pub khz: u32,
    pub bitrate: u16,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub kind: TrackKind,
    pub seekable: bool,
    pub offline: bool,
    /// The file's container, upper-cased ("FLAC"), for a track that has
    /// one. A row from a service names no file, so it has none.
    pub format: Option<String>,
    pub musicbrainz_release_id: Option<String>,
    pub musicbrainz_recording_id: Option<String>,
    pub musicbrainz_track_id: Option<String>,
    pub artwork: Option<crate::ArtworkRef>,
    /// Every credited artist in billing order; `artist` is the billing as the source shows it.
    pub credits: Vec<ArtistCredit>,
}

/// One artist a row credits: what the row calls them, and the key that opens them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtistCredit {
    pub name: String,
    /// `None` for a name the daemon cannot tie to one artist, which nothing opens.
    pub key: Option<String>,
}

impl TrackInfo {
    /// The credit `artist` names in any case, else the lead: credits come in billing order.
    pub fn primary_credit(&self) -> Option<&ArtistCredit> {
        let billed = self.artist.trim();
        self.credits
            .iter()
            .find(|credit| same_name(credit.name.trim(), billed))
            .or_else(|| self.credits.first())
    }
}

fn same_name(a: &str, b: &str) -> bool {
    a.chars()
        .flat_map(char::to_lowercase)
        .eq(b.chars().flat_map(char::to_lowercase))
}

pub const DEFAULT_PAGE_LIMIT: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub offset: u32,
    pub limit: u32,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: DEFAULT_PAGE_LIMIT,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackFilter {
    pub search: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub favorite: Option<bool>,
    pub sort: TrackSort,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LyricChunkView {
    pub start_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LyricLineView {
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub text: String,
    pub chunks: Vec<LyricChunkView>,
    pub parent_line_index: Option<u32>,
    pub background: bool,
    pub opposite_turn: bool,
}

/// Lyrics for one track: `synced` when timing exists (chunks carry word or
/// syllable timing where the provider has it), `plain` otherwise.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LyricsView {
    pub plain: Option<String>,
    pub synced: Vec<LyricLineView>,
}

/// Listening stats: the active source's play counts keyed by track uid.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatsView {
    pub listen_counts: std::collections::HashMap<String, u64>,
}

/// A window into a filtered track listing. `total` always reflects the whole
/// filtered set so clients can paginate without a second count request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackPage {
    pub total: u32,
    pub offset: u32,
    pub items: Vec<TrackInfo>,
}

/// An album row on the wire. `artwork` is present when the daemon can resolve
/// a cover; clients fetch it through [`crate::ArtworkApi`] rather than
/// composing a URL, since the daemon holds the credentials that sign one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumInfo {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub genre: String,
    pub year: u16,
    pub artwork: Option<crate::ArtworkRef>,
    /// Absent only when the album bills nobody.
    pub artist_key: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumPage {
    pub albums: Vec<AlbumInfo>,
    pub total: u32,
}

/// An artist and how many tracks the library holds for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistInfo {
    pub key: String,
    pub name: String,
    pub track_count: u32,
    pub artwork: Option<crate::ArtworkRef>,
}

/// An artist page's header and albums; its tracks page through `artist_tracks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistDetail {
    pub info: ArtistInfo,
    pub albums: Vec<AlbumInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtistPage {
    pub artists: Vec<ArtistInfo>,
    pub total: u32,
}

/// A search. With no `filter` it is the source's plain search, answered with
/// `tracks` and `albums`; with one of [`crate::SourceCapabilities::search_filters`]
/// it is answered with `shelves`. `continuation` asks for more of a filtered
/// search.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchRequest {
    pub query: String,
    pub filter: Option<String>,
    pub continuation: Option<String>,
}

impl SearchRequest {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            ..Self::default()
        }
    }

    pub fn filtered(query: impl Into<String>, filter: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            filter: Some(filter.into()),
            continuation: None,
        }
    }
}

/// What a search turned up. Remote sources answer over the network, so this
/// is one call rather than a filter the caller composes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchResults {
    pub tracks: Vec<TrackInfo>,
    pub albums: Vec<AlbumInfo>,
    /// A filtered search's results: a top result and a shelf per kind for an
    /// "all" filter, one shelf for any other.
    pub shelves: Vec<crate::CatalogShelf>,
    /// More of a filtered search: pass it back in the request, and the
    /// answer's one shelf continues the last one.
    pub continuation: Option<String>,
    /// What the source searched for instead, when it corrected the query.
    pub correction: Option<String>,
}

/// One completion under a search box: a query to run, or, with `item`, a
/// direct hit to open.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchSuggestion {
    pub text: String,
    /// A query the account searched for before.
    pub from_history: bool,
    pub item: Option<crate::CatalogItem>,
}

impl TrackInfo {
    /// Whole seconds, which is how a duration is shown. A radio stream has
    /// none: it plays until it is stopped.
    pub fn duration_secs(&self) -> Option<u64> {
        match self.kind {
            TrackKind::Radio => None,
            TrackKind::Normal => Some(self.duration_ms.unwrap_or_default() / 1000),
        }
    }

    pub fn is_radio(&self) -> bool {
        self.kind == TrackKind::Radio
    }
}

#[cfg(test)]
mod tests {
    use super::{ArtistCredit, TrackInfo};

    fn track(artist: &str, credits: &[(&str, &str)]) -> TrackInfo {
        TrackInfo {
            artist: artist.into(),
            credits: credits
                .iter()
                .map(|(name, key)| ArtistCredit {
                    name: (*name).into(),
                    key: Some((*key).into()),
                })
                .collect(),
            ..Default::default()
        }
    }

    fn primary(row: &TrackInfo) -> Option<&str> {
        row.primary_credit()?.key.as_deref()
    }

    #[test]
    fn the_billed_artist_is_matched_by_name() {
        let row = track("Boris", &[("Ada", "ada"), ("Boris", "b")]);

        assert_eq!(primary(&row), Some("b"));
    }

    #[test]
    fn the_billed_artist_is_matched_in_any_case() {
        let row = track("boris", &[("Ada", "ada"), ("Boris", "b")]);
        assert_eq!(primary(&row), Some("b"));

        let row = track("JÉJA", &[("Cartoon", "c"), ("Jéja", "j")]);
        assert_eq!(primary(&row), Some("j"));
    }

    /// A joined credit names no single artist, so the lead is the one to open.
    #[test]
    fn a_joined_billing_opens_its_lead() {
        let row = track("Ada, Boris", &[("Ada", "ada"), ("Boris", "b")]);

        assert_eq!(primary(&row), Some("ada"));
    }

    /// No string is parsed to find the lead; the source's own order says who it is.
    #[test]
    fn an_unmatched_billing_opens_the_first_credit() {
        let row = track("Ada & Boris", &[("Ada", "ada"), ("Boris", "b")]);

        assert_eq!(primary(&row), Some("ada"));
    }

    #[test]
    fn a_lone_credit_answers_whatever_the_billing_says() {
        let row = track("Ada feat. Boris", &[("Ada", "ada")]);

        assert_eq!(primary(&row), Some("ada"));
    }

    #[test]
    fn a_row_crediting_nobody_opens_nobody() {
        assert_eq!(primary(&track("", &[])), None);
    }
}

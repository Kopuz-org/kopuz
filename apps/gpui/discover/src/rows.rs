//! Which Discover rows are drawn, and whether as content or a placeholder; apart from GPUI so it can be tested.

use api::{ApiError, CatalogShelf, DiscoverRows, ErrorCode, PageEntry, SourceCapabilities};

/// The declared pages that get a row each, by the ids the source declares them under.
pub const ROW_PAGES: &[&str] = &[
    "FEmusic_new_releases",
    "FEmusic_charts",
    "FEmusic_moods_and_genres",
];

/// The source's declared pages that get a row, in the order it declares them.
pub fn row_pages(pages: &[PageEntry]) -> Vec<PageEntry> {
    pages
        .iter()
        .filter(|page| ROW_PAGES.contains(&page.id.as_str()))
        .cloned()
        .collect()
}

/// The shelf a page's row previews: its first one that holds anything.
pub fn first_shelf(shelves: &[CatalogShelf]) -> Option<CatalogShelf> {
    shelves
        .iter()
        .find(|shelf| !shelf.items.is_empty())
        .cloned()
}

/// One read the landing waits on.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Fetch<T> {
    #[default]
    Loading,
    Ready(T),
    Failed(ApiError),
}

impl<T> Fetch<T> {
    fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    /// A daemon from before the call answers every ask as unsupported, which is no failure worth showing.
    fn failure(&self) -> Option<&ApiError> {
        match self {
            Self::Failed(error) if error.code != ErrorCode::Unsupported => Some(error),
            _ => None,
        }
    }
}

/// A row of the landing, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Mixes,
    /// The row for the page at this index of [`row_pages`].
    Page(usize),
    NeverPlayed,
    ForgottenFavorites,
}

/// Whether a row has its content yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    Placeholder,
    Ready,
}

/// What the landing draws under its header.
#[derive(Debug, Clone, PartialEq)]
pub enum Landing {
    Rows(Vec<(Row, Fill)>),
    /// Nothing is drawn yet and nothing is known to come.
    Waiting,
    Empty,
    Failed(ApiError),
}

/// The rows to draw: a placeholder only where the source can fill the row, content only where it did.
pub fn plan(
    caps: &SourceCapabilities,
    rows: &Fetch<DiscoverRows>,
    pages: &[Fetch<Option<CatalogShelf>>],
) -> Vec<(Row, Fill)> {
    let held = |pick: fn(&DiscoverRows) -> bool, possible: bool| match rows {
        Fetch::Loading => possible.then_some(Fill::Placeholder),
        Fetch::Ready(rows) => pick(rows).then_some(Fill::Ready),
        Fetch::Failed(_) => None,
    };
    let mut out = Vec::new();
    if let Some(fill) = held(|rows| !rows.mixes.is_empty(), caps.discover) {
        out.push((Row::Mixes, fill));
    }
    for (index, page) in pages.iter().enumerate() {
        let fill = match page {
            Fetch::Loading => Some(Fill::Placeholder),
            Fetch::Ready(Some(shelf)) if !shelf.items.is_empty() => Some(Fill::Ready),
            Fetch::Ready(_) | Fetch::Failed(_) => None,
        };
        if let Some(fill) = fill {
            out.push((Row::Page(index), fill));
        }
    }
    let local = caps.complete_play_history;
    if let Some(fill) = held(|rows| !rows.never_played.is_empty(), local) {
        out.push((Row::NeverPlayed, fill));
    }
    if let Some(fill) = held(|rows| !rows.forgotten_favorites.is_empty(), local) {
        out.push((Row::ForgottenFavorites, fill));
    }
    out
}

/// The landing as a whole: its rows, or why there are none.
pub fn landing(
    caps: &SourceCapabilities,
    rows: &Fetch<DiscoverRows>,
    pages: &[Fetch<Option<CatalogShelf>>],
) -> Landing {
    let planned = plan(caps, rows, pages);
    if !planned.is_empty() {
        return Landing::Rows(planned);
    }
    if rows.is_loading() || pages.iter().any(Fetch::is_loading) {
        return Landing::Waiting;
    }
    let failure = rows
        .failure()
        .or_else(|| pages.iter().find_map(Fetch::failure));
    match failure {
        Some(error) => Landing::Failed(error.clone()),
        None => Landing::Empty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{AlbumInfo, CatalogItem, TrackInfo};

    fn page(id: &str) -> PageEntry {
        PageEntry {
            id: id.into(),
            ..PageEntry::default()
        }
    }

    fn shelf(title: &str, items: usize) -> CatalogShelf {
        CatalogShelf {
            title: title.into(),
            items: vec![CatalogItem::default(); items],
            ..CatalogShelf::default()
        }
    }

    fn youtube() -> SourceCapabilities {
        SourceCapabilities {
            discover: true,
            ..SourceCapabilities::default()
        }
    }

    fn local() -> SourceCapabilities {
        SourceCapabilities {
            complete_play_history: true,
            ..SourceCapabilities::default()
        }
    }

    fn rows(mixes: usize, never_played: usize, forgotten: usize) -> Fetch<DiscoverRows> {
        Fetch::Ready(DiscoverRows {
            mixes: vec![CatalogItem::default(); mixes],
            never_played: vec![AlbumInfo::default(); never_played],
            forgotten_favorites: vec![TrackInfo::default(); forgotten],
        })
    }

    fn failed() -> ApiError {
        ApiError::new(ErrorCode::SourceUnreachable, "down")
    }

    #[test]
    fn only_new_releases_charts_and_moods_get_a_row_in_declared_order() {
        let declared = [
            page("FEmusic_home"),
            page("FEmusic_explore"),
            page("FEmusic_new_releases"),
            page("FEmusic_charts"),
            page("FEmusic_moods_and_genres"),
            page("FEmusic_home?ggMGSgQIDBAD"),
        ];
        let ids: Vec<String> = row_pages(&declared).into_iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            [
                "FEmusic_new_releases",
                "FEmusic_charts",
                "FEmusic_moods_and_genres"
            ]
        );
    }

    #[test]
    fn a_source_that_declares_none_of_them_gets_no_page_rows() {
        assert!(row_pages(&[page("FEmusic_home"), page("FEmusic_history")]).is_empty());
        assert!(row_pages(&[]).is_empty());
    }

    #[test]
    fn a_page_row_previews_its_first_shelf_that_holds_anything() {
        let shelves = [shelf("empty", 0), shelf("albums", 3), shelf("later", 2)];
        assert_eq!(
            first_shelf(&shelves).map(|s| s.title),
            Some("albums".into())
        );
        assert_eq!(first_shelf(&[shelf("empty", 0)]), None);
    }

    #[test]
    fn placeholders_stand_only_where_the_source_can_fill_the_row() {
        let pages = [Fetch::Loading, Fetch::Loading];
        assert_eq!(
            plan(&youtube(), &Fetch::Loading, &pages),
            [
                (Row::Mixes, Fill::Placeholder),
                (Row::Page(0), Fill::Placeholder),
                (Row::Page(1), Fill::Placeholder),
            ]
        );
        assert_eq!(
            plan(&local(), &Fetch::Loading, &[]),
            [
                (Row::NeverPlayed, Fill::Placeholder),
                (Row::ForgottenFavorites, Fill::Placeholder),
            ]
        );
    }

    #[test]
    fn an_empty_row_hides_itself() {
        assert_eq!(
            plan(&local(), &rows(0, 4, 0), &[]),
            [(Row::NeverPlayed, Fill::Ready)]
        );
        assert_eq!(
            plan(&youtube(), &rows(2, 0, 0), &[]),
            [(Row::Mixes, Fill::Ready)]
        );
    }

    #[test]
    fn a_page_row_hides_when_its_page_is_empty_or_failed() {
        let pages = [
            Fetch::Ready(Some(shelf("albums", 3))),
            Fetch::Ready(None),
            Fetch::Ready(Some(shelf("nothing", 0))),
            Fetch::Failed(failed()),
        ];
        assert_eq!(
            plan(&youtube(), &rows(0, 0, 0), &pages),
            [(Row::Page(0), Fill::Ready)]
        );
    }

    #[test]
    fn the_youtube_rows_never_show_on_a_local_library() {
        let planned = plan(&local(), &rows(0, 1, 1), &[]);
        assert!(
            !planned
                .iter()
                .any(|(row, _)| matches!(row, Row::Mixes | Row::Page(_)))
        );
    }

    #[test]
    fn rows_that_failed_hide_and_the_rest_still_show() {
        let pages = [Fetch::Ready(Some(shelf("charts", 5)))];
        assert_eq!(
            landing(&youtube(), &Fetch::Failed(failed()), &pages),
            Landing::Rows(vec![(Row::Page(0), Fill::Ready)])
        );
    }

    #[test]
    fn nothing_at_all_is_a_calm_empty_state() {
        let caps = SourceCapabilities::default();
        assert_eq!(landing(&caps, &rows(0, 0, 0), &[]), Landing::Empty);
        assert_eq!(
            landing(
                &caps,
                &Fetch::Failed(ApiError::new(ErrorCode::Unsupported, "")),
                &[]
            ),
            Landing::Empty
        );
    }

    #[test]
    fn nothing_but_a_failure_says_so() {
        let pages = [Fetch::Failed(failed())];
        assert_eq!(
            landing(&youtube(), &rows(0, 0, 0), &pages),
            Landing::Failed(failed())
        );
    }

    #[test]
    fn a_read_still_out_with_nothing_to_draw_is_waiting_not_empty() {
        let caps = SourceCapabilities::default();
        assert_eq!(landing(&caps, &Fetch::Loading, &[]), Landing::Waiting);
    }
}

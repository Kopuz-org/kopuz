//! The header controls the two artist pages share: view mode, stacked sort, and Tracks⇄Albums.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use api::{AlbumPage, AlbumQuery, ApiError, ArtistPage, ArtistQuery, KopuzApi};
use components::format::t;
use components::{Segment, Segmented, SortControl};
use config::{
    AlbumSortField, AlbumViewMode, ArtistSortField, ArtistViewOrder, SortCriterion, UiPrefs,
};
use gpui_kit::{App, IntoElement};
use store::LayoutModel;

/// Which pref a view-mode toggle writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewModeKey {
    Artists,
    ArtistAlbums,
}

impl ViewModeKey {
    fn read(self, config: &UiPrefs) -> AlbumViewMode {
        match self {
            Self::Artists => config.artists_view_mode,
            Self::ArtistAlbums => config.artist_album_view_mode,
        }
    }

    fn write(self, config: &mut UiPrefs, mode: AlbumViewMode) {
        match self {
            Self::Artists => config.artists_view_mode = mode,
            Self::ArtistAlbums => config.artist_album_view_mode = mode,
        }
    }
}

/// A sortable field: its label, and the pref the criteria stack lives under.
pub trait SortTarget: Copy + PartialEq + std::fmt::Debug + 'static {
    fn label_key(self) -> &'static str;
    fn read(config: &UiPrefs) -> Vec<SortCriterion<Self>>;
    fn write(config: &mut UiPrefs, criteria: Vec<SortCriterion<Self>>);
}

impl SortTarget for ArtistSortField {
    fn label_key(self) -> &'static str {
        match self {
            Self::Name => "sort_field_name",
            Self::Tracks => "sort_field_tracks",
            Self::Albums => "sort_field_albums",
        }
    }

    fn read(config: &UiPrefs) -> Vec<SortCriterion<Self>> {
        config.artist_sort.clone()
    }

    fn write(config: &mut UiPrefs, criteria: Vec<SortCriterion<Self>>) {
        config.artist_sort = criteria;
    }
}

impl SortTarget for AlbumSortField {
    fn label_key(self) -> &'static str {
        match self {
            Self::Title => "sort_field_title",
            Self::Artist => "sort_field_artist",
            Self::Year => "sort_field_year",
            Self::Genre => "sort_field_genre",
            Self::RecentlyAdded => "sort_field_date_added",
            Self::TrackCount => "sort_field_tracks",
        }
    }

    fn read(config: &UiPrefs) -> Vec<SortCriterion<Self>> {
        config.artist_album_sort.clone()
    }

    fn write(config: &mut UiPrefs, criteria: Vec<SortCriterion<Self>>) {
        config.artist_album_sort = criteria;
    }
}

pub type Listing<T> = Pin<Box<dyn Future<Output = Result<T, ApiError>> + Send>>;

/// Every artist, in the order `criteria` stack up to, sorted by the daemon.
pub fn artist_listing(
    criteria: &[SortCriterion<ArtistSortField>],
) -> impl Fn(Arc<dyn KopuzApi>) -> Listing<ArtistPage> + 'static {
    let query = ArtistQuery {
        sort: criteria.to_vec(),
        ..ArtistQuery::default()
    };
    move |api| {
        let query = query.clone();
        Box::pin(async move { api.artists(query, store::all()).await })
    }
}

/// The albums billed to `artist`, in the order `criteria` stack up to, sorted by the daemon.
pub fn album_listing(
    artist: String,
    criteria: &[SortCriterion<AlbumSortField>],
) -> impl Fn(Arc<dyn KopuzApi>) -> Listing<AlbumPage> + 'static {
    let query = AlbumQuery {
        artist_key: Some(artist),
        sort: criteria.to_vec(),
        ..AlbumQuery::default()
    };
    move |api| {
        let query = query.clone();
        Box::pin(async move { api.albums(query, store::all()).await })
    }
}

/// Read the criteria stack a control edits; the defaults until the daemon's prefs load.
pub fn criteria<F: SortTarget>(cx: &App) -> Vec<SortCriterion<F>> {
    let model = LayoutModel::global(cx);
    F::read(&model.read(cx).get().ui)
}

/// How `key`'s listing is laid out, read off the prefs without copying them.
pub fn view_mode(key: ViewModeKey, cx: &App) -> AlbumViewMode {
    key.read(&LayoutModel::global(cx).read(cx).get().ui)
}

/// Whether an artist's page opens on their albums rather than their tracks.
pub fn albums_first(cx: &App) -> bool {
    LayoutModel::global(cx).read(cx).get().ui.artist_view_order == ArtistViewOrder::Albums
}

/// Grid ⇄ list, writing `key`'s pref.
pub fn view_mode_toggle(key: ViewModeKey, cx: &App) -> impl IntoElement {
    let grid = view_mode(key, cx) == AlbumViewMode::Grid;
    let id = match key {
        ViewModeKey::Artists => "artists-view",
        ViewModeKey::ArtistAlbums => "artist-albums-view",
    };
    components::view_toggle(id, grid, move |grid, cx| {
        let mode = match grid {
            true => AlbumViewMode::Grid,
            false => AlbumViewMode::List,
        };
        LayoutModel::edit(cx, |layout| key.write(&mut layout.ui, mode))
    })
    .pill(true)
}

/// The stacked sort: the first field decides, the rest break ties, name always last.
pub fn sort_control<F: SortTarget>(
    id: &'static str,
    available: Vec<F>,
    cx: &App,
) -> impl IntoElement {
    SortControl::new(id, criteria::<F>(cx))
        .fields(
            available
                .into_iter()
                .map(|field| (field, t(field.label_key()))),
        )
        .on_change(|criteria, cx| {
            LayoutModel::edit(cx, move |layout| F::write(&mut layout.ui, criteria))
        })
}

/// Tracks ⇄ Albums for one artist, writing the `artist_view_order` pref.
pub fn view_order_toggle(albums: bool) -> impl IntoElement {
    Segmented::new("artist-order")
        .pill(true)
        .segment(Segment::new("tracks").label(t("tracks")))
        .segment(Segment::new("albums").label(t("albums")))
        .selected(Some(usize::from(albums)))
        .on_select(|index, _, cx| {
            let order = match index {
                0 => ArtistViewOrder::Tracks,
                _ => ArtistViewOrder::Albums,
            };
            LayoutModel::edit(cx, move |layout| layout.ui.artist_view_order = order)
        })
}

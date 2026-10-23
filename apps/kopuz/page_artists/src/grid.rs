//! The artists grid: every credited name in the library, with the photo the daemon found for it.

use api::{ArtistInfo, ArtistPage, Table};
use components::format::t;
use components::{EmptyState, GridMode, Loading, PageHeader, TileGrid, icons};
use config::{AlbumViewMode, ArtistSortField, SortCriterion};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div, px,
};
use store::{ConfigModel, LayoutModel, Query};

use crate::controls::{self, ViewModeKey};

const TILE: f32 = 160.;

/// The artists grid.
pub struct Artists {
    artists: Entity<Query<ArtistPage>>,
    // The order the listing was last asked for; the daemon sorts, so a new one is a new read.
    ordered: Vec<SortCriterion<ArtistSortField>>,
    grid: TileGrid,
    // Built on change, not per frame: render runs on every playhead tick.
    entries: Vec<ArtistInfo>,
    _subscriptions: Vec<Subscription>,
}

impl Artists {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let ordered = controls::criteria::<ArtistSortField>(cx);
        // An album count is part of each row, so an album change is a change to the listing.
        let artists = cx.new(|cx| {
            Query::new(
                &[Table::Tracks, Table::Albums, Table::Artists],
                controls::artist_listing(&ordered),
                cx,
            )
        });
        let subscriptions = vec![
            cx.observe(&artists, |this, _, cx| this.reread(cx)),
            // The sort and the view mode live in the prefs, so a change there reorders the grid.
            cx.observe(&LayoutModel::global(cx), |this, _, cx| this.reorder(cx)),
            cx.observe(&ConfigModel::global(cx), |_, _, cx| cx.notify()),
        ];
        Self {
            artists,
            ordered,
            // `gap-8` between round tiles.
            grid: TileGrid::new("artists-scroll").gap(px(32.)).fallback(5),
            entries: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    fn reread(&mut self, cx: &mut Context<Self>) {
        // TODO(offline): filter to artists whose tracks are downloaded when offline.
        self.entries = self
            .artists
            .read(cx)
            .value()
            .map(|page| page.artists.clone())
            .unwrap_or_default();
        cx.notify();
    }

    /// Ask the daemon again when the order changed; anything else only repaints.
    fn reorder(&mut self, cx: &mut Context<Self>) {
        let wanted = controls::criteria::<ArtistSortField>(cx);
        if wanted != self.ordered {
            self.artists.update(cx, |query, cx| {
                query.set_fetch(controls::artist_listing(&wanted), cx)
            });
            self.ordered = wanted;
        }
        cx.notify();
    }

    fn is_settled(&self, cx: &App) -> bool {
        self.artists.read(cx).is_settled()
    }
}

impl Render for Artists {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("artists");
        let settled = self.is_settled(cx);
        let mode = match controls::view_mode(ViewModeKey::Artists, cx) {
            AlbumViewMode::Grid => GridMode::Tiles,
            AlbumViewMode::List => GridMode::Rows,
        };
        let empty = self.entries.is_empty();

        let body = match (settled, empty) {
            (false, true) => Loading::new().into_any_element(),
            (true, true) => EmptyState::new(t("artists"))
                .icon(icons::MIC)
                .detail(t("no_tracks_found"))
                .into_any_element(),
            _ => div()
                .flex_1()
                .min_h_0()
                .child(
                    self.grid
                        .bands(
                            self.entries.len(),
                            px(TILE),
                            mode,
                            move |this: &mut Artists, index, _, cx| {
                                let artist = &this.entries[index];
                                match mode {
                                    GridMode::Tiles => components::artist_tile(artist)
                                        .size(px(TILE))
                                        .into_any_element(),
                                    GridMode::Rows => components::artist_row(artist)
                                        .thumb(store::ConfigModel::global(cx).read(cx).row_images())
                                        .into_any_element(),
                                }
                            },
                        )
                        .build(window, cx),
                )
                .into_any_element(),
        };

        components::page()
            .child(
                PageHeader::new(t("artists"))
                    .action(controls::view_mode_toggle(ViewModeKey::Artists, cx))
                    .action(controls::sort_control(
                        "artist-sort",
                        vec![
                            ArtistSortField::Name,
                            ArtistSortField::Tracks,
                            ArtistSortField::Albums,
                        ],
                        cx,
                    )),
            )
            .child(body)
    }
}

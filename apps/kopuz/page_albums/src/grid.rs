//! The albums grid: every album the library holds; two albums sharing a title are two tiles.

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

use api::{AlbumInfo, AlbumPage, AlbumQuery, ApiError, KopuzApi, Table};
use components::album_collection::{available_fields, field_label};
use components::format::t;
use components::{AlbumCollection, EmptyState, GridMode, Loading, PageHeader, SortControl, icons};
use config::{AlbumSortField, AlbumViewMode, SortCriterion};
use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled,
    Subscription, Window, div,
};
use store::{ConfigModel, LayoutModel, Query};

// TODO(offline): filtering the grid to downloaded albums waits on an offline seam the checklist flags as wrong.

/// Every album, as tiles or as rows.
pub struct Albums {
    albums: Entity<Query<AlbumPage>>,
    // The order the listing was last asked for; the daemon sorts, so a new one is a new read.
    asked: Vec<SortCriterion<AlbumSortField>>,
    // Shared once per answer, since a card hover notifies and a band must not clone the listing.
    visible: Rc<[AlbumInfo]>,
    sort_fields: Vec<AlbumSortField>,
    grid: AlbumCollection,
    _subscriptions: Vec<Subscription>,
}

impl Albums {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let asked = criteria(cx);
        let albums = cx.new(|cx| Query::new(&[Table::Albums], listing(&asked), cx));
        let prefs = LayoutModel::global(cx);
        let config = ConfigModel::global(cx);
        let subscriptions = vec![
            cx.observe(&albums, |this, _, cx| this.rebuild(cx)),
            // The sort lives in the prefs, so a change there is a change to the order.
            cx.observe(&prefs, |this, _, cx| this.reorder(cx)),
            cx.observe(&config, |_, _, cx| cx.notify()),
        ];
        Self {
            albums,
            asked,
            visible: Rc::from([]),
            sort_fields: Vec::new(),
            grid: AlbumCollection::new("albums-scroll"),
            _subscriptions: subscriptions,
        }
    }

    /// Ask the daemon again when the order changed; anything else only repaints.
    fn reorder(&mut self, cx: &mut Context<Self>) {
        let wanted = criteria(cx);
        if wanted != self.asked {
            self.albums
                .update(cx, |query, cx| query.set_fetch(listing(&wanted), cx));
            self.asked = wanted;
        }
        cx.notify();
    }

    /// Note which fields are worth offering, on change rather than per frame.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let albums = self.all_albums(cx);
        self.sort_fields = available_fields(&albums, true);
        self.visible = albums.into();
        cx.notify();
    }

    fn all_albums(&self, cx: &App) -> Vec<AlbumInfo> {
        self.albums
            .read(cx)
            .value()
            .map(|page| page.albums.clone())
            .unwrap_or_default()
    }

    fn view_toggle(&self, cx: &App) -> impl IntoElement {
        components::view_toggle(
            "albums-view",
            view_mode(cx) == AlbumViewMode::Grid,
            |grid, cx| {
                let mode = match grid {
                    true => AlbumViewMode::Grid,
                    false => AlbumViewMode::List,
                };
                LayoutModel::edit(cx, |layout| layout.ui.album_view_mode = mode);
            },
        )
    }

    fn sort_control(&self, cx: &App) -> impl IntoElement {
        SortControl::new("albums-sort", criteria(cx))
            .fields(
                self.sort_fields
                    .iter()
                    .map(|field| (*field, field_label(*field))),
            )
            .on_change(|sorted, cx| LayoutModel::edit(cx, |layout| layout.ui.album_sort = sorted))
    }
}

impl Render for Albums {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("albums");
        let settled = self.albums.read(cx).is_settled();
        let mode = match view_mode(cx) {
            AlbumViewMode::Grid => GridMode::Tiles,
            AlbumViewMode::List => GridMode::Rows,
        };
        let empty = self.visible.is_empty();
        let bands = (!empty).then(|| {
            self.grid
                .bands::<Albums>(self.visible.clone(), mode, window, cx)
        });

        components::page()
            .child(PageHeader::new(t("all_albums")))
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .justify_end()
                    .items_center()
                    .gap_2()
                    .mb_4()
                    .child(self.view_toggle(cx))
                    .child(self.sort_control(cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .pb_8()
                    .map(|body| match (bands, settled) {
                        (Some(bands), _) => body.child(bands),
                        (None, false) => body.child(Loading::new()),
                        (None, true) => {
                            body.child(EmptyState::new(t("no_albums_found")).icon(icons::DISC))
                        }
                    }),
            )
    }
}

fn view_mode(cx: &App) -> AlbumViewMode {
    LayoutModel::global(cx).read(cx).get().ui.album_view_mode
}

fn criteria(cx: &App) -> Vec<SortCriterion<AlbumSortField>> {
    LayoutModel::global(cx).read(cx).get().ui.album_sort.clone()
}

type Listing = Pin<Box<dyn Future<Output = Result<AlbumPage, ApiError>> + Send>>;

/// Every album, in the order `criteria` stack up to, sorted by the daemon.
fn listing(
    criteria: &[SortCriterion<AlbumSortField>],
) -> impl Fn(Arc<dyn KopuzApi>) -> Listing + 'static {
    let query = AlbumQuery {
        sort: criteria.to_vec(),
        ..AlbumQuery::default()
    };
    move |api| {
        let query = query.clone();
        Box::pin(async move { api.albums(query, store::all()).await })
    }
}

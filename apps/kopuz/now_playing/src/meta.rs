//! The bar's track block: cover, text and marks, wired to the player; the paint is in `components`.

use components::{
    Cover, MetaLayout, NowPlayingMeta, Picture, TrackMarks, WithContextMenu as _, menus, style,
};
use gpui_kit::base::ElementExt as _;
use gpui_kit::component::h_flex;
use gpui_kit::{
    App, Context, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, rems,
};
use store::{
    Chrome, FavoritesModel, Pictured, PlayerModel, PlaylistsModel, QueueModel, Route, SourcesModel,
    Target, actions,
};

use crate::NowPlaying;

const MARK_GLYPH: Pixels = px(16.);

/// The bar's cover, at whatever size the preference asks for, drawn at the bar's `scale`.
pub(crate) fn cover_size(scale: f32, cx: &App) -> Pixels {
    let size = match store::layout(cx).player.cover {
        store::layout::TileSize::Small => px(40.),
        store::layout::TileSize::Medium => px(56.),
        store::layout::TileSize::Large => px(72.),
    };
    size * scale
}

/// Where the daemon says the queue came from, and what to call it.
pub(crate) fn playing_from(cx: &App) -> Option<(SharedString, Route)> {
    let route = PlayerModel::global(cx)
        .read(cx)
        .queue_context()
        .and_then(Route::playing_from)?;
    let label = match &route {
        Route::Album(id) => QueueModel::global(cx)
            .read(cx)
            .current()
            .map(|track| SharedString::from(track.album.clone()))
            .filter(|album| !album.is_empty())
            .unwrap_or_else(|| id.clone()),
        Route::Artist(artist) => QueueModel::global(cx)
            .read(cx)
            .current()
            .and_then(|track| {
                track
                    .credits
                    .iter()
                    .find(|credit| credit.key.as_ref() == Some(artist))
            })
            .map(|credit| SharedString::from(credit.name.clone()))
            .unwrap_or_default(),
        Route::Genre(name) => name.clone(),
        Route::Playlist(id) => PlaylistsModel::global(cx)
            .read(cx)
            .playlist(id)
            .map(|playlist| SharedString::from(playlist.name.clone()))
            .unwrap_or_else(|| id.clone()),
        _ => return None,
    };
    (!label.is_empty()).then_some((label, route))
}

impl NowPlaying {
    /// The block on the left of the bar: cover, title, artist, where it all came from, and the marks.
    pub(crate) fn track_meta(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let track = self.track(cx);
        let target = track.as_ref().map(track_target);
        let picture = track.as_ref().and_then(track_picture);
        let scale = self.scale(cx);
        let size = cover_size(scale, cx);
        let origin = playing_from(cx);
        let artwork = track.as_ref().and_then(|track| track.artwork.as_ref());
        let swap = self
            .swap
            .frame(components::cover::is_ready(artwork, false, cx), window, cx);
        let cover = |artwork: Option<&api::ArtworkRef>| {
            Cover::new(artwork)
                .size(size)
                .rounded(style::thumb_radius())
        };
        let meta = |id: &'static str, track: &api::TrackInfo| {
            NowPlayingMeta::new(id, track)
                .layout(MetaLayout::Bar)
                .scale(scale)
                .origin(origin.clone())
        };
        let leaving_cover = swap
            .leaving
            .as_ref()
            .filter(|_| swap.cover < 1.)
            .map(|old| {
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .child(cover(old.artwork.as_ref()))
            });
        let title = match (track.as_ref(), swap.leaving.as_ref()) {
            (Some(track), Some(old)) if swap.title < 1. => Some(
                crossfade(meta("bar-leaving", old), meta("bar", track), swap.title)
                    .min_w_0()
                    .into_any_element(),
            ),
            (Some(track), _) => Some(meta("bar", track).into_any_element()),
            (None, _) => None,
        };
        let marks = self
            .marks(
                "meta",
                crate::bar::menu_fits(window, scale),
                track.as_ref(),
                cx,
            )
            .size(style::chrome().round_button * scale, MARK_GLYPH * scale)
            .gap(px(16.) * scale);
        // The heart is the preferences' to keep on the bar; the sheet always has it.
        let marks = match store::layout(cx).player.favorite {
            true => marks,
            false => marks.favorite(None),
        };

        let block = h_flex()
            .id("track-meta")
            .flex_1()
            .min_w_0()
            .gap(rems(scale))
            .child(
                div()
                    .flex_shrink_0()
                    .id("bar-cover")
                    .relative()
                    .on_prepaint(|bounds, _, cx| crate::remember_cover_slot(bounds, cx))
                    .cursor_pointer()
                    .on_click(|_, _, cx| {
                        Chrome::global(cx).update(cx, |chrome, cx| chrome.set_sheet(true, cx))
                    })
                    .children(leaving_cover)
                    .child(
                        div()
                            .opacity(swap.cover)
                            .child(cover(artwork).spot(picture.as_ref().map(Picture::spot))),
                    ),
            )
            .children(title)
            .child(marks);

        block.with_context_menu(menus::pictured(target.map(menus::menu_for), picture))
    }

    /// The marks the preferences and the source allow, acting on `track`; the caller sets their size.
    pub(crate) fn marks(
        &self,
        id: &'static str,
        menu: bool,
        track: Option<&api::TrackInfo>,
        cx: &App,
    ) -> TrackMarks {
        let capabilities = SourcesModel::global(cx).read(cx).capabilities();
        let key = track.map(|track| track.key.clone());
        let favorite = key
            .as_deref()
            .is_some_and(|key| FavoritesModel::global(cx).read(cx).is_favorite(key));
        let (heart, dislike) = (key.clone(), key);

        TrackMarks::new(id)
            .favorite(Some(favorite))
            .dont_recommend(capabilities.dont_recommend)
            .menu(track.map(track_target).filter(|_| menu))
            .disabled(track.is_none())
            .on_favorite(move |_, cx| {
                if let Some(key) = heart.clone() {
                    FavoritesModel::global(cx)
                        .update(cx, |favorites, cx| favorites.toggle(key, cx));
                }
            })
            .on_dont_recommend(move |_, cx| {
                if let Some(key) = dislike.clone() {
                    dont_recommend(key, cx);
                }
            })
    }

    /// Share, for the surfaces that keep it in reach.
    pub(crate) fn share_mark(&self, id: &'static str, cx: &App) -> TrackMarks {
        let key = self.track(cx).map(|track| track.key);
        TrackMarks::new(id)
            .share(true)
            .disabled(key.is_none())
            .on_share(move |_, cx| {
                if let Some(key) = key.clone() {
                    actions::share_track(key, cx);
                }
            })
    }
}

/// The signal is the source's, the skip is ours: it happens whether or not the daemon took it.
pub(crate) fn dont_recommend(key: String, cx: &mut App) {
    let call =
        store::Link::global(cx).call(move |api| async move { api.dont_recommend(key).await });
    cx.spawn(async move |cx| {
        if let Err(error) = call.await {
            cx.update(|cx| store::Notices::failed(&error, cx));
        }
        cx.update(|cx| store::PlayerModel::global(cx).update(cx, |player, cx| player.next(cx)));
    })
    .detach();
}

/// The one thing a `Target` is needed for here, kept out of the render so it allocates once.
pub(crate) fn track_target(track: &api::TrackInfo) -> Target {
    Target::track(track.key.clone(), track.title.clone())
}

/// The playing track's cover as its menus offer it, named `Artist - Title` when saved.
pub(crate) fn track_picture(track: &api::TrackInfo) -> Option<Picture> {
    Picture::new(
        track.artwork.as_ref(),
        Pictured::Track {
            artist: track.artist.clone().into(),
            title: track.title.clone().into(),
        },
    )
}

/// `leaving` fading out where it stood while `arriving` fades in over it, `shown` of the way in.
pub(crate) fn crossfade(
    leaving: impl IntoElement,
    arriving: impl IntoElement,
    shown: f32,
) -> gpui_kit::Div {
    div()
        .relative()
        .flex()
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .right_0()
                .flex()
                .opacity(1. - shown)
                .child(leaving),
        )
        .child(
            div()
                .flex()
                .min_w_0()
                .w_full()
                .opacity(shown)
                .child(arriving),
        )
}

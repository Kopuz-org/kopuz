//! One shelf card: the shared tile, sized for a shelf, with what its play button starts.

use api::QueueContext;
use components::{PlayState, TileRow, style};
use gpui_kit::{AnyElement, App, IntoElement, ParentElement as _, Pixels, Styled as _, div, px};
use store::{PlayerModel, QueueModel, actions};

use crate::data::{Play, Tile};

/// `gap-5` between cards.
pub(crate) const GAP: f32 = 20.;

/// `md:w-48`, which every shelf upstream settles on at desktop width.
pub(crate) const TILE: Pixels = px(192.);

/// The queue's current track, as far as a card's play button cares.
#[derive(Clone, Default)]
pub(crate) struct Now {
    album_id: String,
    key: String,
    playing: bool,
}

impl Now {
    pub(crate) fn read(cx: &App) -> Self {
        let queue = QueueModel::global(cx);
        let Some(track) = queue.read(cx).current() else {
            return Self::default();
        };
        Self {
            album_id: track.album_id.clone(),
            key: track.key.clone(),
            playing: PlayerModel::global(cx).read(cx).is_playing(),
        }
    }

    /// Whether the current track is what `play` names.
    fn is(&self, play: &Play) -> bool {
        match play {
            Play::Album(id) => !self.album_id.is_empty() && &self.album_id == id,
            Play::Track(key) => !self.key.is_empty() && &self.key == key,
            _ => false,
        }
    }
}

pub(crate) fn card(tile: &Tile, now: &Now) -> AnyElement {
    let mut card = components::Tile::new(tile.key.clone(), tile.title.clone())
        .subtitle(tile.subtitle.clone())
        .artwork(tile.artwork.as_ref())
        .size(TILE)
        .round(tile.round)
        .placeholder(tile.placeholder)
        // Upstream only artists and playlists take the accent; an album title stays white.
        .tint(tile.round || matches!(tile.play, Play::Playlist(_)))
        .pictured(tile.pictured.clone())
        .menu(Some(tile.target.clone()));
    if let Some(route) = tile.route.clone() {
        card = card.route(route);
    }
    if let Play::Radio(key) = &tile.play {
        let key = key.clone();
        card = card.on_click(move |_, _, cx| actions::start_track_radio(key.clone(), cx));
    }
    if !matches!(tile.play, Play::None) {
        let current = now.is(&tile.play);
        let state = match current && now.playing {
            true => PlayState::Playing,
            false => PlayState::Idle,
        };
        let play = tile.play.clone();
        card = card.play(state, move |_, _, cx| start(&play, current, cx));
    }
    card.into_any_element()
}

/// The same card as a raised row, for a shelf laid out as a list.
pub(crate) fn row(tile: &Tile, now: &Now, cx: &App) -> AnyElement {
    let current = now.is(&tile.play);
    let mut row = TileRow::new(tile.key.clone(), tile.title.clone())
        .subtitle(tile.subtitle.clone())
        .artwork(tile.artwork.as_ref())
        .placeholder(tile.placeholder)
        .pictured(tile.pictured.clone())
        .playing(current)
        .menu(Some(tile.target.clone()));
    if let Some(route) = tile.route.clone() {
        row = row.route(route);
    }
    if !matches!(tile.play, Play::None) {
        let state = match current && now.playing {
            true => PlayState::Playing,
            false => PlayState::Idle,
        };
        let play = tile.play.clone();
        row = row.play(state, move |_, _, cx| start(&play, current, cx));
    }
    div()
        .min_w_0()
        .rounded(style::radius_xl())
        .border_1()
        .border_color(style::white(style::BORDER_SOFT, cx))
        .bg(style::white(style::RAISED, cx))
        .child(row)
        .into_any_element()
}

fn start(play: &Play, current: bool, cx: &mut App) {
    if current {
        PlayerModel::global(cx).update(cx, |player, cx| player.toggle(cx));
        return;
    }
    match play {
        Play::Album(id) => {
            actions::play_context(QueueContext::Album { id: id.clone() }, Some(0), cx)
        }
        Play::Playlist(id) => {
            actions::play_context(QueueContext::Playlist { id: id.clone() }, Some(0), cx)
        }
        Play::Track(key) => actions::play_tracks(vec![key.clone()], Some(0), cx),
        Play::Radio(key) => actions::start_track_radio(key.clone(), cx),
        Play::None => {}
    }
}

//! Albums, artists and playlists: one virtualized band grid, with a jump rail and type-ahead.

use api::{AlbumInfo, ArtistInfo, PlaylistInfo};
use components::format::t_with;
use components::{Frame, GridMode, JumpRail, TileRow, rail_letter};
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, IntoElement, Pixels, SharedString, Window, px,
};
use store::layout::ViewMode;
use store::{Navigator, Pictured, PlaylistsModel, Route, Target};

use crate::facet::Facet;
use crate::{Library, Tile};

/// Below this, every tile is on screen already and a rail would be decoration.
const RAIL_FROM: usize = 40;

impl Library {
    /// The tiles this facet draws, built once per change rather than once per frame.
    pub(crate) fn rebuild_tiles(&mut self, cx: &mut Context<Self>) {
        let mut tiles = match self.filters().facet {
            Facet::Albums => self.album_tiles(cx),
            Facet::Artists => self.artist_tiles(cx),
            Facet::Playlists => self.playlist_tiles(cx),
            Facet::Tracks => Vec::new(),
        };
        // The daemon orders albums and artists; only the playlist catalog is held and ordered here.
        if self.filters().facet == Facet::Playlists {
            let descending = self.sort().descending;
            let field = self.sort().field.clone();
            tiles.sort_by(|left, right| {
                let order = match field.as_str() {
                    "tracks" => left.sort_rank.cmp(&right.sort_rank),
                    _ => left.sort_key.cmp(&right.sort_key),
                };
                match descending {
                    true => order.reverse(),
                    false => order,
                }
            });
        }
        self.rail_letters = self.whole.then(|| {
            tiles
                .iter()
                .map(|tile| rail_letter(&tile.sort_key))
                .collect()
        });
        self.tiles = tiles;
    }

    fn album_tiles(&self, cx: &Context<Self>) -> Vec<Tile> {
        let Some(page) = self.albums.read(cx).value() else {
            return Vec::new();
        };
        page.albums
            .iter()
            .map(|album| self.album_tile(album))
            .collect()
    }

    fn album_tile(&self, album: &AlbumInfo) -> Tile {
        let playing = self.playing.is_album(&album.id);
        Tile {
            id: SharedString::from(album.id.clone()),
            title: SharedString::from(album.title.clone()),
            subtitle: SharedString::from(album.artist.clone()),
            artwork: album.artwork.clone(),
            sort_key: album.title.to_lowercase(),
            sort_rank: album.year as u64,
            playing,
            target: Target::Album {
                id: album.id.clone().into(),
                title: album.title.clone().into(),
            },
            route: Route::Album(album.id.clone().into()),
            pictured: Pictured::Album {
                artist: album.artist.clone().into(),
                title: album.title.clone().into(),
            },
        }
    }

    fn artist_tiles(&self, cx: &Context<Self>) -> Vec<Tile> {
        let Some(page) = self.artists.read(cx).value() else {
            return Vec::new();
        };
        page.artists
            .iter()
            .map(|artist| self.artist_tile(artist))
            .collect()
    }

    fn artist_tile(&self, artist: &ArtistInfo) -> Tile {
        let playing = self.playing.is_artist(&artist.key);
        Tile {
            // The id is the key, since a selection hands it back to `artist_tracks`.
            id: SharedString::from(artist.key.to_string()),
            title: SharedString::from(artist.name.clone()),
            subtitle: t_with(
                "library_artist_tracks",
                &[("count", artist.track_count.into())],
            ),
            artwork: artist.artwork.clone(),
            sort_key: artist.name.to_lowercase(),
            sort_rank: artist.track_count as u64,
            playing,
            target: Target::Artist {
                key: artist.key.clone(),
                name: artist.name.clone().into(),
            },
            route: Route::Artist(artist.key.clone()),
            pictured: Pictured::Artist(artist.name.clone().into()),
        }
    }

    fn playlist_tiles(&self, cx: &Context<Self>) -> Vec<Tile> {
        let query = self.filters().search.trim().to_lowercase();
        let playing = self.playing.track();
        PlaylistsModel::global(cx)
            .read(cx)
            .playlists()
            .iter()
            .filter(|playlist| query.is_empty() || playlist.name.to_lowercase().contains(&query))
            .map(|playlist| playlist_tile(playlist, playing))
            .collect()
    }

    /// The grid itself: bands of tiles, or one row per entry, beside the jump rail.
    pub(crate) fn grid(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let facet = self.filters().facet;
        let mode = match self.mode {
            ViewMode::Grid => GridMode::Tiles,
            ViewMode::Table => GridMode::Rows,
        };
        let rail = self.jump_rail(cx);
        self.grid
            .bands(
                self.total(),
                px(self.tile.width()),
                mode,
                move |this: &mut Library, index, _, cx| this.cell(index, mode, facet, cx),
            )
            .visit(|this: &mut Library, shown, window, cx| this.ensure_window(&shown, window, cx))
            .rail(rail)
            .build(window, cx)
    }

    /// One entry of the grid, or the shape that stands in for it until its page arrives.
    fn cell(
        &mut self,
        index: usize,
        mode: GridMode,
        facet: Facet,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        profile::elements::built();
        let width = px(self.tile.width());
        let Some(tile) = self.tile_at(index).cloned() else {
            return match mode {
                GridMode::Tiles => {
                    crate::skeleton::tile(width, facet.round_tiles(), cx).into_any_element()
                }
                GridMode::Rows => crate::skeleton::catalog_row(
                    store::ConfigModel::global(cx).read(cx).row_images(),
                    cx,
                )
                .into_any_element(),
            };
        };
        match mode {
            GridMode::Tiles => self.card(&tile, index, width, facet, cx),
            GridMode::Rows => self.line_of(&tile, index, facet, cx),
        }
    }

    fn card(
        &self,
        tile: &Tile,
        index: usize,
        width: Pixels,
        facet: Facet,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        components::Tile::new(tile.id.clone(), tile.title.clone())
            .subtitle(tile.subtitle.clone())
            .artwork(tile.artwork.as_ref())
            .size(width - px(32.))
            .round(facet.round_tiles())
            .placeholder(facet.placeholder())
            .frame(Frame::Card)
            .playing(tile.playing)
            .selected(self.selection.contains(&tile.id))
            .cursor(self.selection.cursor() == Some(index))
            .on_click(self.click_listener(tile.clone(), index, cx))
            .pictured(tile.pictured.clone())
            .menu(Some(tile.target.clone()))
            .into_any_element()
    }

    fn line_of(
        &self,
        tile: &Tile,
        index: usize,
        facet: Facet,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        TileRow::new(tile.id.clone(), tile.title.clone())
            .thumb(store::ConfigModel::global(cx).read(cx).row_images())
            .subtitle(tile.subtitle.clone())
            .artwork(tile.artwork.as_ref())
            .round(facet.round_tiles())
            .placeholder(facet.placeholder())
            .playing(tile.playing)
            .selected(self.selection.contains(&tile.id))
            .on_click(self.click_listener(tile.clone(), index, cx))
            .pictured(tile.pictured.clone())
            .menu(Some(tile.target.clone()))
            .into_any_element()
    }

    /// A plain click opens; ctrl and shift build a selection, the same as in the table.
    fn click_listener(
        &self,
        tile: Tile,
        index: usize,
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this: &mut Library, event: &ClickEvent, window, cx| {
            window.focus(&this.focus, cx);
            let modifiers = event.modifiers();
            let id = tile.id.to_string();
            if modifiers.secondary() {
                this.selection.toggle(&id, index);
            } else if modifiers.shift && this.selection.is_active() {
                let ids: Vec<String> = this.tiles.iter().map(|tile| tile.id.to_string()).collect();
                let offset = this.tile_offset();
                this.selection.extend_to(index, move |slot| {
                    crate::window_item(&ids, offset, slot).cloned()
                });
            } else if this.selection.is_active() {
                this.selection.toggle(&id, index);
            } else {
                this.selection.set_cursor(Some(index));
                Navigator::navigate(tile.route.clone(), cx);
                return;
            }
            cx.notify();
        })
    }

    /// The letters down the right edge, while every tile is held and they are in title order.
    fn jump_rail(&self, cx: &mut Context<Self>) -> Option<JumpRail> {
        if self.total() < RAIL_FROM || self.sort().field != "title" {
            return None;
        }
        let present = self.rail_letters.clone()?;
        let page = cx.entity().downgrade();
        Some(JumpRail::new(
            "library-rail",
            present,
            move |letter, _, cx| {
                page.update(cx, |this, cx| this.jump_to_letter(letter, cx))
                    .ok();
            },
        ))
    }

    fn jump_to_letter(&mut self, letter: char, cx: &mut Context<Self>) {
        let Some(index) = self
            .tiles
            .iter()
            .position(|tile| rail_letter(&tile.sort_key) == letter)
        else {
            return;
        };
        let index = index + self.tile_offset();
        self.selection.set_cursor(Some(index));
        self.jump_to(index);
        cx.notify();
    }
}

fn playlist_tile(playlist: &PlaylistInfo, playing: Option<&str>) -> Tile {
    let playing = playing.is_some_and(|key| playlist.track_keys.iter().any(|held| held == key));
    Tile {
        id: SharedString::from(playlist.id.clone()),
        title: SharedString::from(playlist.name.clone()),
        subtitle: t_with(
            "playlist_track_count",
            &[("count", playlist.track_keys.len().into())],
        ),
        artwork: playlist.artwork.clone(),
        sort_key: playlist.name.to_lowercase(),
        sort_rank: playlist.track_keys.len() as u64,
        playing,
        target: Target::Playlist {
            id: playlist.id.clone().into(),
            name: playlist.name.clone().into(),
        },
        route: Route::Playlist(playlist.id.clone().into()),
        pictured: Pictured::Playlist(playlist.name.clone().into()),
    }
}

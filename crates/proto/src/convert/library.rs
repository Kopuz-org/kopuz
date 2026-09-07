use super::*;
use crate::*;

pub fn track_filter_to_proto(value: &api::TrackFilter) -> TrackFilter {
    TrackFilter {
        search: value.search.clone(),
        artist: value.artist.clone(),
        album: value.album.clone(),
        genre: value.genre.clone(),
        favorite: value.favorite,
        sort: track_sort_to_proto(value.sort) as i32,
    }
}

pub fn track_filter_from_proto(value: &TrackFilter) -> api::TrackFilter {
    api::TrackFilter {
        search: value.search.clone(),
        artist: value.artist.clone(),
        album: value.album.clone(),
        genre: value.genre.clone(),
        favorite: value.favorite,
        sort: track_sort_from_proto(value.sort),
    }
}

pub fn page_to_proto(value: api::Page) -> Page {
    Page {
        offset: value.offset,
        limit: value.limit,
    }
}

pub fn page_from_proto(value: Option<&Page>) -> api::Page {
    let value = value.cloned().unwrap_or_default();
    api::Page {
        offset: value.offset,
        limit: if value.limit == 0 {
            api::DEFAULT_PAGE_LIMIT
        } else {
            value.limit
        },
    }
}

pub fn track_info_to_proto(value: &api::TrackInfo) -> TrackInfo {
    TrackInfo {
        key: value.key.clone(),
        uid: value.uid.clone(),
        title: value.title.clone(),
        artist: value.artist.clone(),
        album: value.album.clone(),
        album_id: value.album_id.clone(),
        duration_ms: value.duration_ms,
        khz: value.khz,
        bitrate: u32::from(value.bitrate),
        track_number: value.track_number,
        disc_number: value.disc_number,
        kind: track_kind_to_proto(value.kind) as i32,
        seekable: value.seekable,
        offline: value.offline,
    }
}

pub fn track_info_from_proto(value: &TrackInfo) -> api::TrackInfo {
    api::TrackInfo {
        key: value.key.clone(),
        uid: value.uid.clone(),
        title: value.title.clone(),
        artist: value.artist.clone(),
        album: value.album.clone(),
        album_id: value.album_id.clone(),
        duration_ms: value.duration_ms,
        khz: value.khz,
        bitrate: value.bitrate.min(u32::from(u16::MAX)) as u16,
        track_number: value.track_number,
        disc_number: value.disc_number,
        kind: track_kind_from_proto(value.kind),
        seekable: value.seekable,
        offline: value.offline,
    }
}

pub fn track_page_to_proto(value: &api::TrackPage) -> TrackPage {
    TrackPage {
        total: value.total,
        offset: value.offset,
        items: value.items.iter().map(track_info_to_proto).collect(),
    }
}

pub fn track_page_from_proto(value: &TrackPage) -> api::TrackPage {
    api::TrackPage {
        total: value.total,
        offset: value.offset,
        items: value.items.iter().map(track_info_from_proto).collect(),
    }
}

pub fn lyrics_to_proto(value: &api::LyricsView) -> Lyrics {
    Lyrics {
        plain: value.plain.clone(),
        synced: value
            .synced
            .iter()
            .map(|line| LyricLine {
                start_ms: line.start_ms,
                end_ms: line.end_ms,
                text: line.text.clone(),
                chunks: line
                    .chunks
                    .iter()
                    .map(|chunk| LyricChunk {
                        start_ms: chunk.start_ms,
                        text: chunk.text.clone(),
                    })
                    .collect(),
                parent_line_index: line.parent_line_index,
                background: line.background,
                opposite_turn: line.opposite_turn,
            })
            .collect(),
    }
}

pub fn lyrics_from_proto(value: &Lyrics) -> api::LyricsView {
    api::LyricsView {
        plain: value.plain.clone(),
        synced: value
            .synced
            .iter()
            .map(|line| api::LyricLineView {
                start_ms: line.start_ms,
                end_ms: line.end_ms,
                text: line.text.clone(),
                chunks: line
                    .chunks
                    .iter()
                    .map(|chunk| api::LyricChunkView {
                        start_ms: chunk.start_ms,
                        text: chunk.text.clone(),
                    })
                    .collect(),
                parent_line_index: line.parent_line_index,
                background: line.background,
                opposite_turn: line.opposite_turn,
            })
            .collect(),
    }
}

pub fn stats_to_proto(value: &api::StatsView) -> Stats {
    Stats {
        listen_counts: value.listen_counts.clone().into_iter().collect(),
    }
}

pub fn stats_from_proto(value: &Stats) -> api::StatsView {
    api::StatsView {
        listen_counts: value.listen_counts.clone().into_iter().collect(),
    }
}

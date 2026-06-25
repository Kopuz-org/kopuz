use std::time::Duration;

use super::{
    ItunesSearchResponse, ItunesSong, LyricChunk, LyricLine, Lyrics, PaxsenixAppleLyricLine,
    PaxsenixAppleLyricPart, PaxsenixAppleLyricsResponse, PaxsenixYoutubeSearchResult,
    has_usable_line_timing, lrc_has_usable_timing, lyrics_kind, lyrics_match_score, parse_lrc,
    timed_line_count, timed_part_count,
};

const PAXSENIX_ROOT_URL: &str = "https://lyrics.paxsenix.org";
const ITUNES_SEARCH_ROOT_URL: &str = "https://itunes.apple.com/search";
const PAXSENIX_TIMEOUT: Duration = Duration::from_secs(5);
const PAXSENIX_APPLE_LYRICS_TIMEOUT: Duration = Duration::from_secs(10);
const PAXSENIX_YOUTUBE_LYRICS_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) async fn fetch_from_paxsenix_youtube(
    artist: &str,
    title: &str,
    duration: u64,
    track_path: &str,
) -> Option<Lyrics> {
    let client = reqwest::Client::new();
    let video_id = if let Some(video_id) = extract_youtube_video_id(track_path) {
        video_id
    } else {
        let query = format!("{title} {artist}");
        let query = query.trim();
        if query.is_empty() {
            return None;
        }

        let results = client
            .get(format!("{PAXSENIX_ROOT_URL}/youtube/search"))
            .query(&[("q", query)])
            .timeout(PAXSENIX_TIMEOUT)
            .send()
            .await
            .map_err(|error| {
                tracing::info!(
                    target: "kopuz::lyrics",
                    "paxsenix_youtube search failed={error}"
                );
            })
            .ok()?
            .json::<Vec<PaxsenixYoutubeSearchResult>>()
            .await
            .map_err(|error| {
                tracing::info!(
                    target: "kopuz::lyrics",
                    "paxsenix_youtube search json_failed={error}"
                );
            })
            .ok()?;

        let selected = best_youtube_result(&results, query, duration)?;
        lyrics_debug!(
            "paxsenix_youtube selected_video id={} title={:?} artist={:?} candidates={}",
            selected.video_id,
            selected.title,
            selected.author,
            results.len()
        );
        selected.video_id.clone()
    };

    let lrc = client
        .get(format!("{PAXSENIX_ROOT_URL}/youtube/lyrics"))
        .query(&[("id", video_id.as_str())])
        .timeout(PAXSENIX_YOUTUBE_LYRICS_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_youtube lyrics failed={error}"
            );
        })
        .ok()?
        .text()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_youtube lyrics text_failed={error}"
            );
        })
        .ok()?;

    if lrc.trim().is_empty() || !lrc_has_usable_timing(&lrc) {
        return None;
    }

    let lines = parse_lrc(&lrc);
    if has_usable_line_timing(&lines) {
        Some(Lyrics::Synced(lines))
    } else {
        None
    }
}

pub(super) fn best_youtube_result<'a>(
    results: &'a [PaxsenixYoutubeSearchResult],
    query: &str,
    duration: u64,
) -> Option<&'a PaxsenixYoutubeSearchResult> {
    results
        .iter()
        .filter_map(|result| {
            let candidate = format!("{} {}", result.title, result.author);
            let text_score = lyrics_match_score(&candidate, query);
            if text_score < 55.0 {
                return None;
            }

            let duration_score = match (duration, parse_colon_duration(&result.duration)) {
                (0, _) | (_, None) => 0.0,
                (expected, Some(candidate_seconds)) => {
                    let delta = candidate_seconds.abs_diff(expected);
                    if delta > 12 {
                        return None;
                    }
                    12.0 - delta as f64
                }
            };

            Some((text_score + duration_score, result))
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, result)| result)
}

pub(super) fn parse_colon_duration(duration: &str) -> Option<u64> {
    let mut total = 0_u64;
    let mut parts = duration.split(':').peekable();
    parts.peek()?;
    for part in parts {
        total = total
            .checked_mul(60)?
            .checked_add(part.parse::<u64>().ok()?)?;
    }
    Some(total)
}

pub(super) fn extract_youtube_video_id(track_path: &str) -> Option<String> {
    track_path
        .strip_prefix("ytmusic:")
        .and_then(|rest| rest.split(':').next())
        .filter(|video_id| !video_id.trim().is_empty())
        .map(|video_id| video_id.to_string())
}

pub(super) async fn fetch_from_paxsenix_apple_music(
    artist: &str,
    title: &str,
    duration: u64,
) -> Option<Lyrics> {
    let query = format!("{title} {artist}");
    let query = query.trim();
    if query.is_empty() {
        return None;
    }

    let client = reqwest::Client::new();
    let search = client
        .get(ITUNES_SEARCH_ROOT_URL)
        .query(&[
            ("term", query),
            ("entity", "song"),
            ("limit", "8"),
            ("country", "US"),
        ])
        .timeout(PAXSENIX_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_apple itunes_search failed={error}"
            );
        })
        .ok()?
        .json::<ItunesSearchResponse>()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_apple itunes_search json_failed={error}"
            );
        })
        .ok()?;

    let Some(song) = best_itunes_song(&search.results, query, duration) else {
        tracing::info!(
            target: "kopuz::lyrics",
            "paxsenix_apple no_match query={query:?} candidates={}",
            search.results.len()
        );
        lyrics_debug!(
            "paxsenix_apple no_match candidates={} query={:?}",
            search.results.len(),
            query
        );
        return None;
    };
    lyrics_debug!(
        "paxsenix_apple selected_track id={} title={:?} artist={:?} candidates={}",
        song.track_id,
        song.track_name,
        song.artist_name,
        search.results.len()
    );

    let response = client
        .get(format!("{PAXSENIX_ROOT_URL}/apple-music/lyrics"))
        .query(&[("id", song.track_id.to_string())])
        .timeout(PAXSENIX_APPLE_LYRICS_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_apple lyrics failed={error}"
            );
        })
        .ok()?
        .json::<PaxsenixAppleLyricsResponse>()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_apple lyrics json_failed={error}"
            );
        })
        .ok()?;

    let lyrics = paxsenix_apple_to_lyrics(response)?;
    if let Lyrics::Synced(lines) = &lyrics {
        lyrics_debug!(
            "paxsenix_apple parsed kind={} lines={} syllable_lines={} syllable_parts={}",
            lyrics_kind(Some(&lyrics)),
            lines.len(),
            timed_line_count(lines),
            timed_part_count(lines)
        );
    } else {
        lyrics_debug!("paxsenix_apple parsed kind={}", lyrics_kind(Some(&lyrics)));
    }
    Some(lyrics)
}

pub(super) fn best_itunes_song<'a>(
    songs: &'a [ItunesSong],
    query: &str,
    duration: u64,
) -> Option<&'a ItunesSong> {
    songs
        .iter()
        .filter_map(|song| {
            let candidate = format!("{} {}", song.track_name, song.artist_name);
            let text_score = lyrics_match_score(&candidate, query);
            if text_score < 55.0 {
                return None;
            }

            let duration_score = match (duration, song.track_time_millis) {
                (0, _) | (_, None) => 0.0,
                (expected, Some(candidate_ms)) => {
                    let candidate_seconds = candidate_ms / 1000;
                    let delta = candidate_seconds.abs_diff(expected);
                    if delta > 12 {
                        return None;
                    }
                    12.0 - delta as f64
                }
            };

            Some((text_score + duration_score, song))
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, song)| song)
}

pub(super) fn paxsenix_apple_to_lyrics(response: PaxsenixAppleLyricsResponse) -> Option<Lyrics> {
    let lines = paxsenix_apple_to_lines(response.content);
    if let Some(lines) = lines
        && has_usable_line_timing(&lines)
    {
        return Some(Lyrics::Synced(lines));
    }

    if let Some(lrc) = response.lrc
        && !lrc.trim().is_empty()
    {
        let parsed = if lrc_has_usable_timing(&lrc) {
            parse_lrc(&lrc)
        } else {
            Vec::new()
        };
        if has_usable_line_timing(&parsed) {
            return Some(Lyrics::Synced(parsed));
        }
    }

    response
        .plain
        .filter(|plain| !plain.trim().is_empty())
        .map(Lyrics::Plain)
}

fn paxsenix_apple_to_lines(rows: Vec<PaxsenixAppleLyricLine>) -> Option<Vec<LyricLine>> {
    if !paxsenix_apple_has_timing(&rows) {
        lyrics_debug!("paxsenix_apple content has no usable timing");
        return None;
    }

    let mut lines = Vec::new();

    for row in rows {
        let row_start_time = row.timestamp as f64 / 1000.0;
        let row_end_time = row.endtime.map(|endtime| endtime as f64 / 1000.0);
        let opposite_turn = row.opposite_turn;
        let row_has_background_text = !row.background_text.is_empty();
        let main_line_is_background = row.background && !row_has_background_text;
        let mut parent_line_index = None;

        if let Some(line) = paxsenix_apple_parts_to_line(
            row.text,
            row_start_time,
            row_end_time,
            None,
            main_line_is_background,
            opposite_turn,
        ) {
            lines.push(line);
            if !main_line_is_background {
                parent_line_index = lines.len().checked_sub(1);
            }
        }

        let background_start_time = row
            .background_text
            .iter()
            .find_map(|part| part.timestamp)
            .map(|timestamp| timestamp as f64 / 1000.0)
            .unwrap_or(row_start_time);

        if let Some(line) = paxsenix_apple_parts_to_line(
            row.background_text,
            background_start_time,
            row_end_time,
            parent_line_index,
            true,
            opposite_turn,
        ) {
            lines.push(line);
        }
    }

    (!lines.is_empty()).then_some(lines)
}

fn paxsenix_apple_has_timing(rows: &[PaxsenixAppleLyricLine]) -> bool {
    rows.iter().any(|row| {
        row.timestamp > 0
            || row.endtime.is_some_and(|endtime| endtime > 0)
            || row
                .text
                .iter()
                .any(|part| part.timestamp.is_some_and(|timestamp| timestamp > 0))
            || row
                .background_text
                .iter()
                .any(|part| part.timestamp.is_some_and(|timestamp| timestamp > 0))
    })
}

fn paxsenix_apple_parts_to_line(
    parts: Vec<PaxsenixAppleLyricPart>,
    start_time: f64,
    end_time: Option<f64>,
    parent_line_index: Option<usize>,
    background: bool,
    opposite_turn: bool,
) -> Option<LyricLine> {
    let mut text = String::new();
    let mut chunks = Vec::new();
    let mut previous_part_continues = false;

    for part in parts {
        if part.text.trim().is_empty() {
            continue;
        }

        let prefix = if should_insert_apple_space(&text, previous_part_continues, &part.text) {
            " "
        } else {
            ""
        };
        let display_text = format!("{prefix}{}", part.text);

        text.push_str(&display_text);
        if let Some(timestamp) = part.timestamp {
            chunks.push(LyricChunk {
                start_time: timestamp as f64 / 1000.0,
                text: display_text,
            });
        }
        previous_part_continues = part.part;
    }

    let text = text.trim().to_string();
    (!text.is_empty()).then_some(LyricLine {
        start_time,
        end_time,
        text,
        chunks,
        parent_line_index,
        background,
        opposite_turn,
    })
}

fn should_insert_apple_space(
    current_text: &str,
    previous_part_continues: bool,
    next_text: &str,
) -> bool {
    if current_text.is_empty() || previous_part_continues {
        return false;
    }

    let Some(first_char) = next_text.chars().next() else {
        return false;
    };

    !matches!(
        first_char,
        ',' | '.' | '?' | '!' | ':' | ';' | ')' | ']' | '}' | '\'' | '’'
    )
}

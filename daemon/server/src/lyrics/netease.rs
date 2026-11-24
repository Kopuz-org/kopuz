//! NetEase Cloud Music: line-timed LRC for most of its catalog, and word-timed
//! `yrc` where it has one. Both are read anonymously through the web API.

use std::time::{Duration, Instant};

use super::{Lyrics, ProviderReach, has_usable_line_timing, lyrics_match_score, parse_lrc};

const SEARCH_URL: &str = "https://music.163.com/api/search/get";
const LYRIC_URL: &str = "https://music.163.com/api/song/lyric/v1";
const NETEASE_TIMEOUT: Duration = Duration::from_secs(5);
// The web API answers a browser; a bare client gets an empty result.
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";
const REFERER: &str = "https://music.163.com/";
/// How far a candidate's length may stray from the track's before it is another recording.
const DURATION_SLACK_MS: u64 = 5_000;
/// Credit lines NetEase puts at the top of a song's lyrics, which are not sung.
const CREDIT_LABELS: &[&str] = &[
    "作词",
    "作曲",
    "编曲",
    "制作人",
    "制作",
    "混音",
    "母带",
    "和声",
    "吉他",
    "贝斯",
    "鼓",
    "弦乐",
    "录音",
    "监制",
    "出品",
    "发行",
    "词",
    "曲",
];

pub(super) async fn fetch_from_netease(
    artist: &str,
    title: &str,
    duration_secs: u64,
    reach: &ProviderReach,
) -> Option<Lyrics> {
    let query = format!("{title} {artist}");
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    let started = Instant::now();
    let search = get(
        SEARCH_URL,
        &[("s", query), ("type", "1"), ("limit", "5")],
        reach,
    )
    .await?;
    let songs = search.pointer("/result/songs")?.as_array()?;
    let Some(id) = best_song_id(songs, query, duration_secs * 1000) else {
        tracing::info!(
            target: "kopuz::lyrics",
            "netease no_match query={query:?} candidates={}",
            songs.len()
        );
        return None;
    };
    let id = id.to_string();
    let lyric = get(
        LYRIC_URL,
        &[("id", id.as_str()), ("lv", "1"), ("yv", "1")],
        reach,
    )
    .await?;
    tracing::info!(
        target: "kopuz::lyrics",
        "netease request elapsed_ms={}",
        started.elapsed().as_millis()
    );

    // Word timing first; the plain LRC is the fallback every song with lyrics has.
    if let Some(yrc) = lyric.pointer("/yrc/lyric").and_then(|v| v.as_str())
        && let Some(enhanced) = yrc_to_enhanced_lrc(yrc)
    {
        let lines = parse_lrc(&enhanced);
        if has_usable_line_timing(&lines) {
            return Some(Lyrics::Synced(lines));
        }
    }
    let lrc = lyric.pointer("/lrc/lyric").and_then(|v| v.as_str())?;
    let lines = parse_lrc(&without_credits(lrc));
    has_usable_line_timing(&lines).then_some(Lyrics::Synced(lines))
}

async fn get(
    url: &str,
    query: &[(&str, &str)],
    reach: &ProviderReach,
) -> Option<serde_json::Value> {
    let response = utils::http::client()
        .get(url)
        .query(query)
        .header("User-Agent", USER_AGENT)
        .header("Referer", REFERER)
        .timeout(NETEASE_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(target: "kopuz::lyrics", "netease request failed={}", error.without_url());
            reach.unreachable();
        })
        .ok()?;
    let value = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| {
            tracing::warn!(target: "kopuz::lyrics", "netease json_failed={}", error.without_url());
        })
        .ok()?;
    (value.get("code").and_then(|code| code.as_i64()) == Some(200)).then_some(value)
}

/// The candidate whose name and artists match the query best, when it matches well enough
/// and, if the track's length is known, is the same recording.
fn best_song_id(songs: &[serde_json::Value], query: &str, duration_ms: u64) -> Option<i64> {
    songs
        .iter()
        .filter_map(|song| {
            let name = song.get("name")?.as_str()?;
            let artists = song
                .get("artists")?
                .as_array()?
                .iter()
                .filter_map(|artist| artist.get("name")?.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let length = song.get("duration").and_then(|d| d.as_u64()).unwrap_or(0);
            if duration_ms > 0 && length > 0 && length.abs_diff(duration_ms) > DURATION_SLACK_MS {
                return None;
            }
            let score = lyrics_match_score(&format!("{name} {artists}"), query);
            Some((score, song.get("id")?.as_i64()?))
        })
        // A tie goes to the earlier candidate: NetEase lists its best guess first.
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .and_then(|(score, id)| (score >= 65.0).then_some(id))
}

/// `yrc` lines read `[start,length](start,length,0)word(start,length,0)word…` in milliseconds;
/// the JSON lines between them are credits. Rewritten as enhanced LRC, which the parser reads.
fn yrc_to_enhanced_lrc(yrc: &str) -> Option<String> {
    let mut output = String::new();
    for line in yrc.lines() {
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((head, mut words)) = rest.split_once(']') else {
            continue;
        };
        let Some(start) = head
            .split(',')
            .next()
            .and_then(|s| s.trim().parse::<u64>().ok())
        else {
            continue;
        };
        let mut line_out = format!("[{}]", lrc_time(start));
        let mut any = false;
        while let Some(open) = words.strip_prefix('(') {
            let Some((timing, after)) = open.split_once(')') else {
                break;
            };
            let Some(at) = timing
                .split(',')
                .next()
                .and_then(|s| s.trim().parse::<u64>().ok())
            else {
                break;
            };
            let end = after.find('(').unwrap_or(after.len());
            let word = &after[..end];
            if !word.is_empty() {
                line_out.push('<');
                line_out.push_str(&lrc_time(at));
                line_out.push('>');
                line_out.push_str(word);
                any = true;
            }
            words = &after[end..];
        }
        if any {
            output.push_str(&line_out);
            output.push('\n');
        }
    }
    (!output.is_empty()).then_some(output)
}

fn lrc_time(ms: u64) -> String {
    let centiseconds = ms / 10;
    format!(
        "{:02}:{:02}.{:02}",
        centiseconds / 6000,
        (centiseconds / 100) % 60,
        centiseconds % 100
    )
}

/// The LRC with the credit lines at its top taken out, so they are not shown as sung words.
fn without_credits(lrc: &str) -> String {
    lrc.lines()
        .filter(|line| {
            let text = line.rsplit(']').next().unwrap_or(line).trim();
            !CREDIT_LABELS.iter().any(|label| {
                text.strip_prefix(label)
                    .is_some_and(|rest| rest.trim_start().starts_with([':', '：']))
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const YRC: &str = r#"{"t":0,"c":[{"tx":"作词: "},{"tx":"Sevänen"}]}
[53880,2910](53880,960,0)Old (54840,390,0)is (55230,450,0)my (55680,1110,0)moon
[56940,2850](56940,1050,0)Circle (57990,540,0)is (58530,1260,0)complete"#;

    #[test]
    fn yrc_becomes_word_timed_lines() {
        let lines = parse_lrc(&yrc_to_enhanced_lrc(YRC).expect("lines"));
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].chunks.len(), 4);
        assert_eq!(lines[0].chunks[0].text, "Old ");
        assert_eq!(lines[1].chunks[2].text, "complete");
        assert!((lines[0].start_time - 53.88).abs() < 0.01);
        assert!((lines[0].chunks[1].start_time - 54.84).abs() < 0.01);
    }

    #[test]
    fn the_credit_lines_are_left_out() {
        let lrc = "[00:00.00] 作词 : Someone\n[00:01.00] 作曲：Other\n[00:53.88]Old is my moon";
        assert_eq!(without_credits(lrc), "[00:53.88]Old is my moon");
    }

    #[test]
    fn a_different_recording_is_not_taken() {
        let songs = serde_json::json!([
            {"id": 1, "name": "The River", "artists": [{"name": "Insomnium"}], "duration": 215_000},
            {"id": 2, "name": "The River", "artists": [{"name": "Insomnium"}], "duration": 477_000},
        ]);
        let songs = songs.as_array().expect("array");
        assert_eq!(best_song_id(songs, "The River Insomnium", 477_000), Some(2));
        assert_eq!(best_song_id(songs, "The River Insomnium", 0), Some(1));
        assert_eq!(
            best_song_id(songs, "Something Else Entirely", 477_000),
            None
        );
    }
}

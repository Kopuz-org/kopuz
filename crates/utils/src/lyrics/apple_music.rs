//! Apple Music TTML lyrics parser.
//!
//! Converts TTML (Timed Text Markup Language) fetched from the Apple Music
//! amp-api into the app's [`Lyrics`] model. Follows the reference
//! implementation in `am-downloader` closely — two code paths based on the
//! `itunes:timing` attribute:
//!
//! - `"Word"` → syllable/word-level timing → [`LyricLine`] with per-word
//!   [`LyricChunk`] entries (quality 2).
//! - `"None"` or missing → plain unsynchronised text (quality 0).
//! - Default (no `itunes:timing` but has `begin` attributes) → line-level
//!   timing (quality 1).

use quick_xml::events::Event;
use quick_xml::Reader;


/// Check if a qualified name matches a local name (ignoring namespace).
fn qname_local_eq(qn: quick_xml::name::QName, local: &[u8]) -> bool {
    qn.local_name().as_ref() == local || qn.as_ref() == local
}
use super::model::{LyricChunk, LyricLine, Lyrics};

/// Parse a TTML string from Apple Music into [`Lyrics`].
///
/// Returns `None` when the TTML is empty, malformed, or contains no usable
/// lyrics content.
pub fn parse_ttml(ttml: &str) -> Option<Lyrics> {
    if ttml.trim().is_empty() {
        return None;
    }
    // Determine the timing mode from the root <tt> element's itunes:timing attr.
    let timing = detect_timing_mode(ttml);
    match timing {
        TimingMode::Word => parse_word_timed(ttml),
        TimingMode::None => parse_plain(ttml),
        TimingMode::Line => parse_line_timed(ttml),
    }
}

/// Lazily fetch Apple Music lyrics from the amp-api. Called from the lyrics
/// provider chain — never pre-fetched in the UI. Makes direct HTTP calls
/// to avoid depending on the server crate.
pub(super) async fn fetch_apple_music_lyrics(
    auth: &super::request::AppleMusicLyricsAuth,
) -> Option<Lyrics> {
    const AM_BASE: &str = "https://amp-api.music.apple.com";
    const AM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

    let client = reqwest::Client::new();

    // Resolve library ID to catalog ID if needed.
    let catalog_id = resolve_catalog_id(&client, auth).await;
    let catalog_id = match catalog_id {
        Some(id) => id,
        None => {
            tracing::debug!("am.lyrics: failed to resolve catalog id for {}", auth.catalog_id);
            return None;
        }
    };

    // Try syllable-lyrics first (word-level timing, quality 2), then lyrics (line-level).
    for lrc_type in &["syllable-lyrics", "lyrics"] {
        let path = format!(
            "/v1/catalog/{}/songs/{}/{lrc_type}?l={}&extend=ttmlLocalizations",
            auth.storefront, catalog_id, auth.language
        );
        let url = format!("{AM_BASE}{path}");
        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", auth.bearer_token))
            .header("User-Agent", USER_AGENT)
            .header("Origin", "https://music.apple.com")
            .header("Referer", "https://music.apple.com/")
            .header("Cookie", format!("media-user-token={}", auth.token))
            .timeout(AM_TIMEOUT)
            .send()
            .await
            .ok();

        let resp = match resp {
            Some(r) if r.status().is_success() => r,
            Some(r) => {
                tracing::debug!(
                    "am.lyrics: {lrc_type} → HTTP {} for {}",
                    r.status(),
                    catalog_id
                );
                continue;
            }
            None => {
                tracing::debug!(
                    "am.lyrics: {lrc_type} → request failed for {}",
                    catalog_id
                );
                continue;
            }
        };

        let body: serde_json::Value = match resp.json().await {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!("am.lyrics: {lrc_type} → json parse error for {}: {e}", catalog_id);
                continue;
            }
        };
        let ttml = body
            .pointer("/data/0/attributes/ttml")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                body.pointer("/data/0/attributes/ttmlLocalizations")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
            });
        let ttml = match ttml {
            Some(t) => t,
            None => {
                tracing::debug!(
                    "am.lyrics: {lrc_type} → no ttml in response for {} (keys: {:?})",
                    catalog_id,
                    body.pointer("/data/0/attributes").and_then(|v| v.as_object()).map(|m| m.keys().collect::<Vec<_>>())
                );
                continue;
            }
        };

        if let Some(lyrics) = parse_ttml(ttml) {
            tracing::debug!(
                "am.lyrics: got {lrc_type} for {} ({} bytes)",
                catalog_id,
                ttml.len()
            );
            return Some(lyrics);
        }
    }
    None
}

/// Resolve a library ID (`i.xxx`) to a catalog ID (numeric).
/// Returns the ID unchanged if it's already numeric.
async fn resolve_catalog_id(
    client: &reqwest::Client,
    auth: &super::request::AppleMusicLyricsAuth,
) -> Option<String> {
    if auth.catalog_id.chars().all(|c| c.is_ascii_digit()) {
        return Some(auth.catalog_id.clone());
    }
    tracing::debug!("am.lyrics: resolving library id {}", auth.catalog_id);
    let path = format!(
        "/v1/me/library/songs/{}/catalog?l={}",
        auth.catalog_id, auth.language
    );
    let url = format!("https://amp-api.music.apple.com{path}");
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", auth.bearer_token))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .header("Origin", "https://music.apple.com")
        .header("Referer", "https://music.apple.com/")
        .header("Cookie", format!("media-user-token={}", auth.token))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    body["data"]
        .as_array()?
        .first()?
        .get("id")?
        .as_str()
        .map(|s| s.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimingMode {
    Word,
    Line,
    None,
}

fn detect_timing_mode(ttml: &str) -> TimingMode {
    // Look for itunes:timing attribute on the root <tt> element.
    // quick-xml doesn't handle namespace prefixes well for custom attrs,
    // so we do a simple search.
    if ttml.contains("itunes:timing=\"Word\"") || ttml.contains("timing=\"Word\"") {
        TimingMode::Word
    } else if ttml.contains("itunes:timing=\"None\"") || ttml.contains("timing=\"None\"") {
        TimingMode::None
    } else {
        TimingMode::Line
    }
}

// ── Plain (untimed) ───────────────────────────────────────────────────

fn parse_plain(ttml: &str) -> Option<Lyrics> {
    let lines = extract_p_texts(ttml);
    if lines.is_empty() {
        return None;
    }
    let text = lines.join("\n");
    if text.trim().is_empty() {
        return None;
    }
    Some(Lyrics::Plain(text))
}

/// Extract the text content of all `<p>` elements.
fn extract_p_texts(ttml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(ttml);
    let mut texts = Vec::new();
    let mut in_p = false;
    let mut buf = Vec::new();
    let mut current_text = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                if qname_local_eq(e.name(), b"p") {
                    in_p = true;
                    current_text.clear();
                }
            }
            Ok(Event::Text(ref e)) if in_p => {
                if let Ok(t) = e.unescape() {
                    current_text.push_str(&t);
                }
            }
            Ok(Event::End(ref e)) => {
                if qname_local_eq(e.name(), b"p") && in_p {
                    let trimmed = current_text.trim().to_string();
                    if !trimmed.is_empty() {
                        texts.push(trimmed);
                    }
                    in_p = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    texts
}

// ── Line-timed ────────────────────────────────────────────────────────

fn parse_line_timed(ttml: &str) -> Option<Lyrics> {
    let lines = parse_line_timed_impl(ttml);
    if lines.is_empty() {
        return None;
    }
    Some(Lyrics::Synced(lines))
}

fn parse_line_timed_impl(ttml: &str) -> Vec<LyricLine> {
    let mut reader = Reader::from_str(ttml);
    let mut lines = Vec::new();
    let mut in_p = false;
    let mut p_begin: Option<String> = None;
    let mut current_text = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                if qname_local_eq(e.name(), b"p") {
                    in_p = true;
                    p_begin = get_attr_value(e, b"begin");
                    current_text.clear();
                }
            }
            Ok(Event::Text(ref e)) if in_p => {
                if let Ok(t) = e.unescape() {
                    current_text.push_str(&t);
                }
            }
            Ok(Event::End(ref e)) => {
                if qname_local_eq(e.name(), b"p") && in_p {
                    let text = current_text.trim().to_string();
                    if !text.is_empty() {
                        if let Some(begin) = &p_begin {
                            if let Some(start_time) = parse_am_time(begin) {
                                lines.push(LyricLine {
                                    start_time,
                                    end_time: None,
                                    text,
                                    chunks: Vec::new(),
                                    parent_line_index: None,
                                    background: false,
                                    opposite_turn: false,
                                });
                            }
                        }
                    }
                    in_p = false;
                    p_begin = None;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    lines
}

// ── Word-timed (syllable) ─────────────────────────────────────────────

fn parse_word_timed(ttml: &str) -> Option<Lyrics> {
    let lines = parse_word_timed_impl(ttml);
    if lines.is_empty() {
        return None;
    }
    Some(Lyrics::Synced(lines))
}

fn parse_word_timed_impl(ttml: &str) -> Vec<LyricLine> {
    let mut reader = Reader::from_str(ttml);
    let mut lines = Vec::new();

    let mut in_p = false;
    let mut in_span = false;
    let mut span_begin: Option<String> = None;
    let mut span_end: Option<String> = None;
    let mut current_text = String::new();
    let mut line_words: Vec<(f64, String)> = Vec::new();
    let mut line_text_parts: Vec<String> = Vec::new();
    let mut span_count: usize = 0;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                if qname_local_eq(e.name(), b"p") {
                    in_p = true;
                    line_words.clear();
                    line_text_parts.clear();
                    span_count = 0;
                } else if qname_local_eq(e.name(), b"span") && in_p {
                    // Insert space before this word if it's not the first span in the line.
                    if span_count > 0 {
                        if let Some(last) = line_text_parts.last_mut() {
                            last.push(' ');
                        }
                    }
                    in_span = true;
                    span_begin = get_attr_value(e, b"begin");
                    span_end = get_attr_value(e, b"end");
                    current_text.clear();
                }
            }
            Ok(Event::Text(ref e)) if in_span => {
                if let Ok(t) = e.unescape() {
                    current_text.push_str(&t);
                }
            }
            Ok(Event::End(ref e)) => {
                if qname_local_eq(e.name(), b"span") && in_span {
                    let text = current_text.trim().to_string();
                    if !text.is_empty() {
                        if let (Some(begin), Some(_end)) = (&span_begin, &span_end) {
                            if let Some(start) = parse_am_time(begin) {
                                line_words.push((start, text.clone()));
                                line_text_parts.push(text);
                                span_count += 1;
                            }
                        }
                    }
                    in_span = false;
                    span_begin = None;
                    span_end = None;
                } else if qname_local_eq(e.name(), b"p") && in_p {
                    if !line_words.is_empty() {
                        let line_text = line_text_parts.concat();
                        let start_time = line_words.first().map(|w| w.0).unwrap_or(0.0);
                        let chunks: Vec<LyricChunk> = line_words
                            .iter()
                            .enumerate()
                            .map(|(idx, (t, text))| LyricChunk {
                                start_time: *t,
                                text: if idx > 0 {
                                    format!(" {text}")
                                } else {
                                    text.clone()
                                },
                            })
                            .collect();
                        lines.push(LyricLine {
                            start_time,
                            end_time: None,
                            text: line_text,
                            chunks,
                            parent_line_index: None,
                            background: false,
                            opposite_turn: false,
                        });
                    }
                    in_p = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    lines
}

// ── Helpers ───────────────────────────────────────────────────────────

fn get_attr_value(e: &quick_xml::events::BytesStart, name: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.local_name().as_ref() == name {
            return String::from_utf8(attr.value.to_vec()).ok();
        }
    }
    None
}

/// Parse an Apple Music time string into seconds (f64).
///
/// Formats: `"mm:ss.xx"`, `"hh:mm:ss.xx"`, `"ss.xx"`, `"ss"`.
fn parse_am_time(value: &str) -> Option<f64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if let Some(colon_pos) = value.rfind(':') {
        let time_part = &value[colon_pos + 1..];
        let prefix = &value[..colon_pos];

        if let Some(dot_pos) = time_part.find('.') {
            let secs: u32 = time_part[..dot_pos].parse().ok()?;
            let frac: u32 = time_part[dot_pos + 1..].parse().ok()?;
            let frac_secs = frac as f64 / 10f64.powi(time_part[dot_pos + 1..].len() as i32);

            if prefix.contains(':') {
                // hh:mm:ss.xx
                let h_m: Vec<&str> = prefix.split(':').collect();
                if h_m.len() == 2 {
                    let h: u32 = h_m[0].parse().ok()?;
                    let m: u32 = h_m[1].parse().ok()?;
                    Some((h as f64 * 3600.0) + (m as f64 * 60.0) + secs as f64 + frac_secs)
                } else {
                    None
                }
            } else {
                // mm:ss.xx
                let m: u32 = prefix.parse().ok()?;
                Some((m as f64 * 60.0) + secs as f64 + frac_secs)
            }
        } else {
            // mm:ss or hh:mm:ss (no fractional part)
            let secs: u32 = time_part.parse().ok()?;
            if prefix.contains(':') {
                let h_m: Vec<&str> = prefix.split(':').collect();
                if h_m.len() == 2 {
                    let h: u32 = h_m[0].parse().ok()?;
                    let m: u32 = h_m[1].parse().ok()?;
                    Some((h as f64 * 3600.0) + (m as f64 * 60.0) + secs as f64)
                } else {
                    None
                }
            } else {
                let m: u32 = prefix.parse().ok()?;
                Some((m as f64 * 60.0) + secs as f64)
            }
        }
    } else if let Some(dot_pos) = value.find('.') {
        // ss.xx
        let secs: u32 = value[..dot_pos].parse().ok()?;
        let frac: u32 = value[dot_pos + 1..].parse().ok()?;
        let frac_secs = frac as f64 / 10f64.powi(value[dot_pos + 1..].len() as i32);
        Some(secs as f64 + frac_secs)
    } else {
        // ss
        value.parse::<u32>().ok().map(|s| s as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_am_time_simple() {
        assert!((parse_am_time("01:23.45").unwrap() - 83.45).abs() < 0.001);
    }

    #[test]
    fn parse_am_time_hours() {
        assert!((parse_am_time("1:02:03.45").unwrap() - 3723.45).abs() < 0.001);
    }

    #[test]
    fn parse_am_time_seconds_only() {
        assert!((parse_am_time("42.5").unwrap() - 42.5).abs() < 0.001);
    }

    #[test]
    fn parse_plain_ttml() {
        let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml"
            xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"
            itunes:timing="None">
            <body><div>
                <p>Hello world</p>
                <p>Second line</p>
            </div></body>
        </tt>"#;
        let result = parse_ttml(ttml).unwrap();
        match result {
            Lyrics::Plain(text) => {
                assert!(text.contains("Hello world"));
                assert!(text.contains("Second line"));
            }
            _ => panic!("expected plain lyrics"),
        }
    }

    #[test]
    fn parse_line_timed_ttml() {
        let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml"
            xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">
            <body><div>
                <p begin="00:01.00">First line</p>
                <p begin="00:05.50">Second line</p>
            </div></body>
        </tt>"#;
        let result = parse_ttml(ttml).unwrap();
        match result {
            Lyrics::Synced(lines) => {
                assert_eq!(lines.len(), 2);
                assert!((lines[0].start_time - 1.0).abs() < 0.001);
                assert_eq!(lines[0].text, "First line");
                assert!((lines[1].start_time - 5.5).abs() < 0.001);
                assert_eq!(lines[1].text, "Second line");
            }
            _ => panic!("expected synced lyrics"),
        }
    }

    #[test]
    fn parse_word_timed_ttml() {
        let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml"
            xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"
            itunes:timing="Word">
            <body><div>
                <p>
                    <span begin="00:01.00" end="00:01.50">Hello</span>
                    <span begin="00:01.60" end="00:02.00">world</span>
                </p>
            </div></body>
        </tt>"#;
        let result = parse_ttml(ttml).unwrap();
        match result {
            Lyrics::Synced(lines) => {
                assert_eq!(lines.len(), 1);
                assert!((lines[0].start_time - 1.0).abs() < 0.001);
                assert_eq!(lines[0].text, "Hello world");
                assert_eq!(lines[0].chunks.len(), 2);
                assert!((lines[0].chunks[0].start_time - 1.0).abs() < 0.001);
                assert_eq!(lines[0].chunks[0].text, "Hello");
                assert!((lines[0].chunks[1].start_time - 1.6).abs() < 0.001);
                assert_eq!(lines[0].chunks[1].text, " world");
            }
            _ => panic!("expected word-synced lyrics"),
        }
    }

    #[test]
    fn parse_empty_ttml() {
        assert!(parse_ttml("").is_none());
        assert!(parse_ttml("   ").is_none());
    }

    #[test]
    fn parse_real_syllable_ttml() {
        // Real Apple Music syllable lyrics from "Paralyzed" by Sleep Theory
        let ttml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:itunes="http://music.apple.com/lyric-ttml-internal" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" itunes:timing="Word" xml:lang="en"><head><metadata><ttm:agent type="person" xml:id="v1"/><iTunesMetadata xmlns="http://music.apple.com/lyric-ttml-internal" leadingSilence="0.140"><translations/><songwriters><songwriter>Ben Pruitt</songwriter></songwriters></iTunesMetadata></metadata></head><body dur="3:26.426"><div begin="33.848" end="44.197" itunes:songPart="Verse"><p begin="33.848" end="35.283" itunes:key="L1" ttm:agent="v1"><span begin="33.848" end="34.101">You</span> <span begin="34.101" end="34.577">feel</span> <span begin="34.577" end="35.283">it</span></p><p begin="35.382" end="36.515" itunes:key="L2" ttm:agent="v1"><span begin="35.382" end="35.885">Creep</span> <span begin="35.885" end="36.515">in</span></p><p begin="36.580" end="39.115" itunes:key="L3" ttm:agent="v1"><span begin="36.580" end="36.821">A</span> <span begin="36.821" end="37.475">thousand</span> <span begin="37.475" end="37.868">knives</span> <span begin="37.868" end="38.076">that</span> <span begin="38.076" end="38.516">sink</span> <span begin="38.516" end="39.115">in</span></p></div></body></tt>"#;
        let result = parse_ttml(ttml).unwrap();
        match result {
            Lyrics::Synced(lines) => {
                assert_eq!(lines.len(), 3);
                // Line 1: "You feel it"
                assert!((lines[0].start_time - 33.848).abs() < 0.001);
                assert_eq!(lines[0].text, "You feel it");
                assert_eq!(lines[0].chunks.len(), 3);
                assert_eq!(lines[0].chunks[0].text, "You");
                assert_eq!(lines[0].chunks[1].text, " feel");
                assert_eq!(lines[0].chunks[2].text, " it");
                // Line 2: "Creep in"
                assert_eq!(lines[1].text, "Creep in");
                assert_eq!(lines[1].chunks.len(), 2);
                // Line 3: "A thousand knives that sink in"
                assert_eq!(lines[2].text, "A thousand knives that sink in");
                assert_eq!(lines[2].chunks.len(), 6);
            }
            _ => panic!("expected synced lyrics"),
        }
    }

}

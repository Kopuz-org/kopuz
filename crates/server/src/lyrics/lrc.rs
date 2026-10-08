use super::{LyricChunk, LyricLine};

fn append_translation(target: &mut String, text: &str) {
    let text = text.trim();

    if text.is_empty() {
        return;
    }

    if !target.is_empty() {
        target.push('\n');
    }

    if text.starts_with('(') && text.ends_with(')') {
        target.push_str(text);
    } else {
        target.push('(');
        target.push_str(text);
        target.push(')');
    }
}

fn append_line_translation(target: &mut LyricLine, text: &str) {
    append_translation(&mut target.text, text);
}

pub(super) fn parse_lrc(lrc_text: &str) -> Vec<LyricLine> {
    let mut lines: Vec<LyricLine> = Vec::new();

    for raw_line in lrc_text.lines() {
        let (line_timestamps, content) = extract_line_timestamps(raw_line);
        let (text, words) = parse_enhanced_words(content);

        if line_timestamps.is_empty() {
            let is_metadata_tag = raw_line.trim().starts_with('[')
                && raw_line.trim().ends_with(']')
                && raw_line.contains(':');
            if is_metadata_tag {
                continue;
            }

            if !words.is_empty() {
                lines.push(LyricLine {
                    start_time: words[0].start_time,
                    end_time: None,
                    text,
                    chunks: words,
                    parent_line_index: None,
                    background: false,
                    opposite_turn: false,
                });
            } else if !text.is_empty()
                && let Some(last) = lines.last_mut()
            {
                append_line_translation(last, &text);
            }
            continue;
        }

        if text.is_empty() && words.is_empty() {
            continue;
        }

        for start_time in line_timestamps {
            lines.push(LyricLine {
                start_time,
                end_time: None,
                text: text.clone(),
                chunks: words.clone(),
                parent_line_index: None,
                background: false,
                opposite_turn: false,
            });
        }
    }

    lines.sort_by(|a, b| {
        a.start_time
            .partial_cmp(&b.start_time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut merged: Vec<LyricLine> = Vec::new();

    for mut line in lines {
        if let Some(last) = merged.last_mut()
            && last.start_time == line.start_time
        {
            if last.chunks.is_empty() && !line.chunks.is_empty() {
                last.chunks = std::mem::take(&mut line.chunks);
            }
            if last.end_time.is_none() {
                last.end_time = line.end_time;
            }
            append_line_translation(last, &line.text);
            continue;
        }

        merged.push(line);
    }

    merged
}

pub(super) fn extract_line_timestamps(line: &str) -> (Vec<f64>, &str) {
    let mut timestamps = Vec::new();
    let mut rest = line.trim_start();

    while let Some(after_open) = rest.strip_prefix('[') {
        let Some(close_idx) = after_open.find(']') else {
            break;
        };

        let tag = &after_open[..close_idx];
        let after_tag = &after_open[close_idx + 1..];
        if let Some(time) = parse_lrc_time(tag) {
            timestamps.push(time);
            rest = after_tag;
        } else if tag.chars().any(|c| c.is_ascii_alphabetic()) && tag.contains(':') {
            rest = after_tag;
        } else {
            break;
        }
    }

    (timestamps, rest)
}

pub(super) fn parse_enhanced_words(content: &str) -> (String, Vec<LyricChunk>) {
    let mut words = Vec::new();
    let mut text = String::new();
    let mut rest = content;
    let mut pending_time: Option<f64> = None;

    while let Some(open_idx) = rest.find('<') {
        let before = &rest[..open_idx];
        text.push_str(before);
        if let Some(start_time) = pending_time.take()
            && !before.is_empty()
        {
            words.push(LyricChunk {
                start_time,
                text: before.to_string(),
            });
        }

        let after_open = &rest[open_idx + 1..];
        let Some(close_idx) = after_open.find('>') else {
            text.push_str(&rest[open_idx..]);
            rest = "";
            break;
        };

        let tag = &after_open[..close_idx];
        if let Some(time) = parse_lrc_time(tag) {
            pending_time = Some(time);
            rest = &after_open[close_idx + 1..];
        } else {
            text.push('<');
            text.push_str(tag);
            text.push('>');
            rest = &after_open[close_idx + 1..];
        }
    }

    text.push_str(rest);
    if let Some(start_time) = pending_time
        && !rest.is_empty()
    {
        words.push(LyricChunk {
            start_time,
            text: rest.to_string(),
        });
    }

    (text.trim().to_string(), words)
}

fn parse_lrc_time(time_str: &str) -> Option<f64> {
    let parts: Vec<&str> = time_str.split([':', '.']).collect();
    if parts.len() >= 2 {
        let min = parts[0].parse::<f64>().ok()?;
        let sec = parts[1].parse::<f64>().ok()?;
        let mut total = min * 60.0 + sec;
        if parts.len() == 3 {
            let ms_str = parts[2];
            let ms = ms_str.parse::<f64>().ok()?;
            let divisor = 10_f64.powi(ms_str.len() as i32);
            total += ms / divisor;
        }
        Some(total)
    } else {
        None
    }
}

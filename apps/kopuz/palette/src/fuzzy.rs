//! Subsequence scoring: what puts one row above another.

/// A run of characters matched back to back.
const CONSECUTIVE: i32 = 40;
/// Per character skipped between two matches.
const GAP: i32 = 4;
/// Per character skipped before the first match.
const LEAD: i32 = 2;
/// A match at the start of a word.
const BOUNDARY: i32 = 60;
/// The whole title is what was typed.
const EXACT: i32 = 1000;
/// The title starts with what was typed.
const PREFIX: i32 = 400;
/// How much a long title is discounted, so the shorter of two matches wins.
const LENGTH: i32 = 4;
/// How far a single gap is allowed to count against a match.
const WORST_GAP: usize = 8;

/// How well `needle` matches `haystack`, or nothing when it does not match at all.
pub fn score(haystack: &str, needle: &str) -> Option<i32> {
    if needle.trim().is_empty() {
        return Some(0);
    }
    let raw: Vec<char> = haystack.chars().collect();
    let hay: Vec<char> = raw.iter().copied().map(lower).collect();
    let pin: Vec<char> = needle
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(lower)
        .collect();
    if pin.is_empty() {
        return Some(0);
    }
    let mut total = 0;
    let mut from = 0;
    let mut previous: Option<usize> = None;
    for wanted in &pin {
        let at = hay[from..].iter().position(|held| held == wanted)? + from;
        total += match previous {
            Some(last) if at == last + 1 => CONSECUTIVE,
            Some(last) => -GAP * (at - last - 1).min(WORST_GAP) as i32,
            None => -LEAD * at.min(WORST_GAP) as i32,
        };
        if is_boundary(&raw, at) {
            total += BOUNDARY;
        }
        previous = Some(at);
        from = at + 1;
    }
    let typed: Vec<char> = needle.trim().chars().map(lower).collect();
    if hay == typed {
        total += EXACT;
    } else if hay.starts_with(&typed) {
        total += PREFIX;
    }
    Some(total - raw.len() as i32 / LENGTH)
}

fn lower(held: char) -> char {
    held.to_lowercase().next().unwrap_or(held)
}

fn is_boundary(raw: &[char], at: usize) -> bool {
    let Some(previous) = at.checked_sub(1).and_then(|before| raw.get(before)) else {
        return true;
    };
    !previous.is_alphanumeric() || (previous.is_lowercase() && raw[at].is_uppercase())
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn nothing_typed_matches_everything_equally() {
        assert_eq!(score("Play Next", ""), Some(0));
        assert_eq!(score("Shuffle", "   "), Some(0));
    }

    #[test]
    fn a_letter_that_is_not_there_is_not_a_match() {
        assert!(score("Play", "xyz").is_none());
        assert!(score("Play", "ply").is_some());
    }

    #[test]
    fn the_whole_word_beats_a_word_it_only_starts() {
        let exact = score("Play", "play").expect("match");
        let prefix = score("Playlists", "play").expect("match");
        assert!(exact > prefix, "{exact} should beat {prefix}");
    }

    #[test]
    fn a_word_start_beats_the_middle_of_a_word() {
        let start = score("Play Next", "pl").expect("match");
        let middle = score("Display", "pl").expect("match");
        assert!(start > middle, "{start} should beat {middle}");
    }

    #[test]
    fn initials_reach_a_two_word_command() {
        let initials = score("Play Next", "pn").expect("match");
        assert!(initials > 0, "initials scored {initials}");
    }

    #[test]
    fn the_shorter_of_two_equal_matches_wins() {
        let short = score("Queue", "queue").expect("match");
        let long = score("Queue Something Else Entirely", "queue").expect("match");
        assert!(short > long, "{short} should beat {long}");
    }

    #[test]
    fn case_is_ignored_both_ways() {
        assert_eq!(score("Shuffle", "SHUF"), score("shuffle", "shuf"));
    }
}

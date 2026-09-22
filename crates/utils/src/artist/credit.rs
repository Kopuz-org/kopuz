//! Splitting a joined artist credit into the artists it actually credits.
//!
//! A tag reading "A$AP Rocky feat. Drake" is one string, and taking it as one
//! artist gives the collaboration its own tile beside the real A$AP Rocky,
//! usually with no photo, because no such artist exists upstream.
//!
//! The rule is deliberately narrow. A wrong split invents phantom artists *and*
//! can lose the real one, which is worse than the duplicate it set out to fix,
//! so only markers that never sit inside a single name are separators here.
//! `&`, `+`, ` - ` and an unpadded slash never are: "&ME", "Simon &
//! Garfunkel", "AC/DC", "Jay-Z" and "Florence + the Machine" survive intact.
//!
//! Three more shapes look like joins but cannot be judged from the string at
//! all, because real names use them too: a comma ("Tyler, The Creator"), a
//! semicolon ("We;Na", "Kairon; IRSE!") and a space-padded slash ("R!N /
//! Gemie", "LOONA / ODD EYE CIRCLE"). Padding does not settle the slash case,
//! so none of them is split here. [`join_candidates`] offers them up instead,
//! and the caller decides with evidence this module cannot see: whether the
//! pieces stand alone as whole credits elsewhere in the library.

/// Featuring markers, longest first so "featuring" wins over its own "feat"
/// prefix.
const FEATURE_MARKERS: [&str; 5] = ["featuring", "feat.", "feat", "ft.", "ft"];

/// Collaboration markers: co-equal credits rather than a featured guest list.
const COLLAB_MARKERS: [&str; 3] = ["vs.", "vs", "x"];

/// Words that open the tail of one name ("Tyler, The Creator") rather than the
/// next entry in a guest list.
const CONTINUATION_WORDS: [&str; 3] = ["the", "a", "an"];

/// The ID3v2.4 / Vorbis multi-value separator and its CJK equivalents. A
/// tagger writes these between values it considers separate, but "We;Na" and
/// "Kairon; IRSE!" are each one artist, so they are candidates rather than
/// separators.
const LIST_DELIMITERS: [char; 3] = [';', '；', '、'];

/// Bullets, which some taggers use to join a whole contributor list. Counted
/// only when padded with a space on both sides: unpadded, every one of these
/// turns up inside real names (Catalan "Col·lectiu", Japanese
/// "マイケル・ジャクソン").
const BULLETS: [char; 5] = ['•', '∙', '·', '・', '･'];

/// A slash can separate co-equal credits, but only with a space against it.
/// "AC/DC" has none and is never a candidate; "A$AP Rocky/ Joe Fox" and "R!N /
/// Gemie" both are, and only the library can say which is a join.
const SLASHES: [char; 1] = ['/'];

/// The individual artists a credit string names.
///
/// Returns the input as a single entry when nothing marks it as a join, and
/// de-duplicates case-insensitively while keeping the first spelling seen.
pub fn split_credit(credit: &str) -> Vec<String> {
    let mut out = Vec::new();
    split_part(credit, &mut out);
    out
}

/// The artists a track credits, preferring the source's own per-artist values
/// (a multi-value `ARTISTS` tag, Jellyfin's `Artists` array) over the joined
/// display string. Each value still goes through [`split_credit`], because a
/// multi-value field can hold one joined credit per slot.
pub fn credited(primary: &str, structured: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in structured {
        split_part(value, &mut out);
    }
    if out.is_empty() {
        split_part(primary, &mut out);
    }
    out
}

/// The key two spellings of one artist share.
pub fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The pieces a credit could be split into, or None when nothing in it even
/// looks like a join.
///
/// A comma, a semicolon and a space-padded slash all join credits and all sit
/// inside real names, and nothing in the string tells the cases apart:
/// "Tyler, The Creator" against "49th & Main, SHEE", "We;Na" against "Daft
/// Punk;Pharrell Williams", "R!N / Gemie" against "A$AP Rocky/ Joe Fox". So
/// this only offers the candidates; the caller decides with evidence this
/// module cannot see, namely whether each piece stands alone as a whole credit
/// elsewhere in the library.
///
/// An unpadded slash is not a candidate at all, which is what keeps "AC/DC"
/// away from the question entirely.
pub fn join_candidates(credit: &str) -> Option<Vec<&str>> {
    let mut pieces = Vec::new();
    let mut start = 0;
    for (i, ch) in credit.char_indices() {
        let end = i + ch.len_utf8();
        let is_boundary = if ch == ',' || LIST_DELIMITERS.contains(&ch) {
            true
        } else if SLASHES.contains(&ch) {
            credit[..i].ends_with(char::is_whitespace)
                || credit[end..].starts_with(char::is_whitespace)
        } else {
            false
        };
        if is_boundary {
            pieces.push(&credit[start..i]);
            start = end;
        }
    }
    if pieces.is_empty() {
        return None;
    }
    pieces.push(&credit[start..]);
    let pieces: Vec<&str> = pieces
        .into_iter()
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
        .collect();
    (pieces.len() > 1).then_some(pieces)
}

fn split_part(part: &str, out: &mut Vec<String>) {
    let part = part.trim();
    if part.is_empty() {
        return;
    }
    // Bullets bind loosest of all: they join whole credits, so they have to
    // be resolved before any marker sitting inside one of those credits.
    if let Some(segments) = bullet_segments(part) {
        push_bulleted(&segments, out);
        return;
    }
    // Collab markers bind looser than "feat.": each side of an "A x B" can
    // carry its own featured list.
    if let Some((start, end)) = find_marker(part, &COLLAB_MARKERS, true) {
        split_part(&part[..start], out);
        split_part(&part[end..], out);
        return;
    }
    match find_marker(part, &FEATURE_MARKERS, false) {
        Some((start, end)) => {
            split_part(&part[..start], out);
            push_guests(&part[end..], out);
        }
        None => push_name(part, out),
    }
}

/// The segments of a bullet-joined list, or None when no bullet carries the
/// space on both sides that tells a separator apart from a character inside a
/// name.
fn bullet_segments(part: &str) -> Option<Vec<&str>> {
    let mut segments = Vec::new();
    let mut start = 0;
    for (i, ch) in part.char_indices() {
        if !BULLETS.contains(&ch) {
            continue;
        }
        let end = i + ch.len_utf8();
        if !part[..i].ends_with(char::is_whitespace)
            || !part[end..].starts_with(char::is_whitespace)
        {
            continue;
        }
        segments.push(&part[start..i]);
        start = end;
    }
    if segments.is_empty() {
        return None;
    }
    segments.push(&part[start..]);
    Some(segments)
}

/// A bullet list is the performing credit followed by everyone who worked on
/// the release: songwriters, producers, and the performers' own legal names.
/// Only the head is what the track files under, so the tail is dropped.
///
/// This does lose a genuine second performer where a tagger used a bullet to
/// join two of them. That is the cheaper mistake. The tail is where "A$AP
/// Rocky" acquires a permanent twin tile reading "Rakim Mayers", and a twin for
/// the same human is the duplicate this whole rule set exists to remove.
///
/// Unlike a comma, a semicolon or a padded slash, this stays a decision the
/// string makes on its own. Handing it to the library would ask the opposite
/// question: not "do the pieces stand alone" but "does the head", and an
/// artist whose every track carries a personnel list answers no, which would
/// hand the whole dump back as the tile. The padding requirement already
/// excludes the shapes that put one of these characters inside a name
/// ("Col·lectiu", "マイケル・ジャクソン"), and a space-padded bullet is not
/// something a real name does.
fn push_bulleted(segments: &[&str], out: &mut Vec<String>) {
    if let Some(head) = segments.first() {
        split_part(head, out);
    }
}

/// Everything after a featuring marker is a guest list, so a comma or an
/// ampersand in it is a separator. That inference holds only here: at the top
/// level the same characters are ordinary parts of a band name.
fn push_guests(tail: &str, out: &mut Vec<String>) {
    for segment in comma_segments(tail) {
        for name in segment.split(" & ").flat_map(|n| n.split(" and ")) {
            split_part(name, out);
        }
    }
}

fn comma_segments(tail: &str) -> Vec<String> {
    let mut segments: Vec<String> = Vec::new();
    for raw in tail.split(',') {
        let piece = raw.trim();
        if piece.is_empty() {
            continue;
        }
        if opens_continuation(piece)
            && let Some(last) = segments.last_mut()
        {
            last.push_str(", ");
            last.push_str(piece);
            continue;
        }
        segments.push(piece.to_string());
    }
    segments
}

fn opens_continuation(piece: &str) -> bool {
    piece.split_whitespace().next().is_some_and(|word| {
        CONTINUATION_WORDS
            .iter()
            .any(|w| word.eq_ignore_ascii_case(w))
    })
}

fn push_name(name: &str, out: &mut Vec<String>) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    if !out.iter().any(|seen| same_artist(seen, name)) {
        out.push(name.to_string());
    }
}

fn same_artist(a: &str, b: &str) -> bool {
    name_key(a) == name_key(b)
}

/// The byte range of the first separator marker in `part`.
///
/// A marker only separates when a space precedes it, so "Taylor Swift." keeps
/// the "ft." inside "Swift" and "Jay-Z" is never touched. It must also be
/// followed by a space, unless it ends in a period: "かいりきベア feat.缶缶"
/// has none there. `require_space_after` additionally rejects a marker at the
/// very end of the string, which is what keeps "Malcolm X" whole.
fn find_marker(part: &str, markers: &[&str], require_space_after: bool) -> Option<(usize, usize)> {
    let bytes = part.as_bytes();
    for (i, ch) in part.char_indices() {
        if i == 0 || !part[..i].ends_with(char::is_whitespace) || ch.is_whitespace() {
            continue;
        }
        for marker in markers {
            let end = i + marker.len();
            if end > bytes.len() || !bytes[i..end].eq_ignore_ascii_case(marker.as_bytes()) {
                continue;
            }
            let after_ok = match part[end..].chars().next() {
                Some(next) => next.is_whitespace(),
                None => !require_space_after,
            } || (marker.ends_with('.') && end < bytes.len());
            if after_ok {
                return Some((i, end));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(s: &str) -> Vec<String> {
        split_credit(s)
    }

    #[test]
    fn plain_names_pass_through() {
        assert_eq!(split("A$AP Rocky"), ["A$AP Rocky"]);
        assert_eq!(split("  Reol  "), ["Reol"]);
        assert_eq!(split(""), Vec::<String>::new());
    }

    #[test]
    fn featuring_markers_split() {
        assert_eq!(split("A$AP Rocky feat. Drake"), ["A$AP Rocky", "Drake"]);
        assert_eq!(split("A$AP Rocky Feat. Drake"), ["A$AP Rocky", "Drake"]);
        assert_eq!(split("A$AP Rocky FEAT Drake"), ["A$AP Rocky", "Drake"]);
        assert_eq!(split("A$AP Rocky ft. Drake"), ["A$AP Rocky", "Drake"]);
        assert_eq!(split("A$AP Rocky ft Drake"), ["A$AP Rocky", "Drake"]);
        assert_eq!(split("A$AP Rocky featuring Drake"), ["A$AP Rocky", "Drake"]);
    }

    #[test]
    fn featuring_marker_without_trailing_space_splits() {
        assert_eq!(split("かいりきベア feat.缶缶"), ["かいりきベア", "缶缶"]);
    }

    #[test]
    fn guest_list_splits_on_comma_and_ampersand() {
        assert_eq!(
            split("Kanye West feat. Jay-Z, Rihanna & Bon Iver"),
            ["Kanye West", "Jay-Z", "Rihanna", "Bon Iver"]
        );
        assert_eq!(
            split("Calvin Harris ft. Dua Lipa and Young Thug"),
            ["Calvin Harris", "Dua Lipa", "Young Thug"]
        );
    }

    #[test]
    fn guest_list_keeps_a_trailing_article_with_its_name() {
        assert_eq!(
            split("Kali Uchis feat. Tyler, The Creator"),
            ["Kali Uchis", "Tyler, The Creator"]
        );
    }

    #[test]
    fn collaboration_markers_split() {
        assert_eq!(split("Chris Brown x Tyga"), ["Chris Brown", "Tyga"]);
        assert_eq!(split("Metallica vs. Slayer"), ["Metallica", "Slayer"]);
        assert_eq!(split("Metallica vs Slayer"), ["Metallica", "Slayer"]);
    }

    #[test]
    /// Semicolons and space-padded slashes are left for the library to rule on,
    /// because real names use both. The splitter hands them over whole.
    fn list_delimiters_are_left_to_the_library() {
        for credit in [
            "Daft Punk;Pharrell Williams",
            "初音ミク、鏡音リン",
            "We;Na",
            "Kairon; IRSE!",
            "R!N / Gemie",
            "A$AP Rocky/ Joe Fox",
        ] {
            assert_eq!(
                split(credit),
                [credit],
                "{credit:?} is not the splitter's call"
            );
        }
    }

    #[test]
    /// What the splitter offers the library, and what it refuses to offer.
    fn join_candidates_offers_every_ambiguous_shape() {
        assert_eq!(join_candidates("We;Na"), Some(vec!["We", "Na"]));
        assert_eq!(
            join_candidates("Kairon; IRSE!"),
            Some(vec!["Kairon", "IRSE!"])
        );
        assert_eq!(join_candidates("R!N / Gemie"), Some(vec!["R!N", "Gemie"]));
        assert_eq!(
            join_candidates("LOONA / ODD EYE CIRCLE"),
            Some(vec!["LOONA", "ODD EYE CIRCLE"])
        );
        assert_eq!(
            join_candidates("A$AP Rocky/ Joe Fox"),
            Some(vec!["A$AP Rocky", "Joe Fox"])
        );
        assert_eq!(
            join_candidates("Tyler, The Creator"),
            Some(vec!["Tyler", "The Creator"])
        );
        // An unpadded slash never becomes a question in the first place.
        assert_eq!(join_candidates("AC/DC"), None);
        assert_eq!(join_candidates("Jay-Z"), None);
        assert_eq!(join_candidates("&ME"), None);
    }

    // The names a split must never shatter. Each is a single real artist whose
    // name contains a character or word that looks like a join.
    #[test]
    fn real_names_are_never_split() {
        for name in [
            "&ME",
            "Simon & Garfunkel",
            "AC/DC",
            "Tyler, The Creator",
            "Earth, Wind & Fire",
            "Florence + the Machine",
            "Jay-Z",
            "Blink-182",
            "Malcolm X",
            "Taylor Swift.",
            "MYTH & ROID",
            "Emerson, Lake & Palmer",
            "Sleeping With Sirens",
            "Nothing But Thieves",
            "Crosby, Stills & Nash",
            "塞壬唱片-MSR",
            "AC/DC",
            "We;Na",
            "Kairon; IRSE!",
            "R!N / Gemie",
            "LOONA / yyxy",
            "LOONA / ODD EYE CIRCLE",
            "Hall & Oates",
            "Godspeed You! Black Emperor",
        ] {
            assert_eq!(split(name), [name], "must not split {name:?}");
        }
    }

    #[test]
    fn a_real_name_still_splits_off_its_features() {
        assert_eq!(
            split("Earth, Wind & Fire feat. The Emotions"),
            ["Earth, Wind & Fire", "The Emotions"]
        );
        assert_eq!(split("Jay-Z ft. Alicia Keys"), ["Jay-Z", "Alicia Keys"]);
    }

    // The shapes below are taken verbatim from a real library. The tail of a
    // bullet list is personnel, so only the head survives: "Rakim Mayers" is
    // A$AP Rocky's legal name, and "Hector Delgado" and "Joe Fox" are credited
    // writers. Each was showing up as its own artist tile.
    #[test]
    fn a_bullet_list_keeps_only_its_head() {
        assert_eq!(split("A$AP Rocky • Rakim Mayers"), ["A$AP Rocky"]);
        assert_eq!(
            split(
                "A$AP Rocky • Bones • Frans Mernick • Hector Delgado • Rakim Mayers • \
                 Elmo O'Connor"
            ),
            ["A$AP Rocky"]
        );
        assert_eq!(
            split("A$AP Rocky • Joe Fox • Rakim Mayers • Brian Burton • Ben Nichols"),
            ["A$AP Rocky"]
        );
    }

    /// The head is split on its own markers, so a genuine collaboration written
    /// before the personnel list survives it.
    #[test]
    fn a_credit_followed_by_its_contributor_list_keeps_only_the_credit() {
        assert_eq!(
            split(
                "A$AP Rocky feat. Joe Fox x Future x M.I.A. • A$AP Rocky • Joe Fox • Future • \
                 M.I.A. • Rakim Mayers • Rameses Magnus-George • Axel Morgan • Ricci Rierra • \
                 Nayvadius Wilburn"
            ),
            ["A$AP Rocky", "Joe Fox", "Future", "M.I.A."]
        );
    }

    /// The accepted cost of head-only: a bullet genuinely joining two
    /// performers loses the second. A featured artist on a handful of tracks is
    /// worth less than never showing one human under two tiles.
    #[test]
    fn a_bullet_between_two_performers_still_loses_the_second() {
        assert_eq!(split("Above & Beyond • Zoë Johnston"), ["Above & Beyond"]);
    }

    /// A padded slash is a candidate wherever the space falls, and a repeated
    /// piece is offered once.
    #[test]
    fn a_padded_slash_is_a_candidate_from_either_side() {
        assert_eq!(
            join_candidates("A$AP Rocky/ James Fauntleroy/ James Fauntleroy"),
            Some(vec!["A$AP Rocky", "James Fauntleroy", "James Fauntleroy"])
        );
        assert_eq!(
            join_candidates("Above & Beyond / Justine Suissa"),
            Some(vec!["Above & Beyond", "Justine Suissa"])
        );
        assert_eq!(
            join_candidates("Zeds Dead /Diplo"),
            Some(vec!["Zeds Dead", "Diplo"])
        );
    }

    #[test]
    fn bullets_inside_a_name_are_left_alone() {
        // Unpadded, these characters are part of the name itself.
        for name in ["マイケル・ジャクソン", "Col·lectiu", "A•B"] {
            assert_eq!(split(name), [name], "must not split {name:?}");
        }
    }

    /// The other bullet characters are recognised too, so their tail is dropped
    /// rather than surviving as one long tile.
    #[test]
    fn other_bullet_shapes_are_recognised() {
        assert_eq!(
            split("Ayumi Hamasaki ・ Tetsuya Komuro"),
            ["Ayumi Hamasaki"]
        );
        assert_eq!(split("Nujabes · Shing02"), ["Nujabes"]);
        assert_eq!(split("Nujabes ∙ Shing02"), ["Nujabes"]);
    }

    #[test]
    fn duplicates_collapse_case_insensitively() {
        assert_eq!(split("Drake feat. drake"), ["Drake"]);
        assert_eq!(split("Drake ft. Future x drake"), ["Drake", "Future"]);
    }

    #[test]
    fn credited_prefers_structured_values() {
        let structured = ["A$AP Rocky".to_string(), "Drake".to_string()];
        assert_eq!(
            credited("A$AP Rocky feat. Drake", &structured),
            ["A$AP Rocky", "Drake"]
        );
    }

    #[test]
    fn credited_splits_a_joined_structured_value() {
        let structured = ["A$AP Rocky feat. Drake".to_string()];
        assert_eq!(credited("whatever", &structured), ["A$AP Rocky", "Drake"]);
    }

    #[test]
    fn credited_falls_back_to_the_display_string() {
        assert_eq!(
            credited("A$AP Rocky feat. Drake", &[]),
            ["A$AP Rocky", "Drake"]
        );
        assert_eq!(credited("", &[]), Vec::<String>::new());
    }
}

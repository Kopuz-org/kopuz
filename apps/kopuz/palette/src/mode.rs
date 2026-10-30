//! What the leading character asks for, and which sections that leaves.

/// What the typed prefix narrows the palette to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Everything,
    Commands,
    Artists,
    Albums,
    Playlists,
}

/// The prefix that selects each mode, in the order they are offered.
pub const PREFIXES: &[(char, Mode)] = &[
    ('>', Mode::Commands),
    ('@', Mode::Artists),
    ('#', Mode::Albums),
    (':', Mode::Playlists),
];

impl Mode {
    /// The i18n key naming this mode in the chip beside the caret.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Everything => "search",
            Mode::Commands => "palette_mode_commands",
            Mode::Artists => "palette_mode_artists",
            Mode::Albums => "palette_mode_albums",
            Mode::Playlists => "palette_mode_playlists",
        }
    }
}

/// Split what was typed into the mode it asks for and the rest of the query.
pub fn parse(typed: &str) -> (Mode, &str) {
    let trimmed = typed.trim_start();
    for (glyph, mode) in PREFIXES {
        if let Some(rest) = trimmed.strip_prefix(*glyph) {
            return (*mode, rest.trim());
        }
    }
    (Mode::Everything, typed.trim())
}

/// One block of rows under its own heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    CurrentTrack,
    RecentCommands,
    RecentSearches,
    Commands,
    Tracks,
    Albums,
    Artists,
    Playlists,
    Navigate,
    Settings,
}

impl Part {
    pub fn label(self) -> &'static str {
        match self {
            Part::CurrentTrack => "palette_current_track",
            Part::RecentCommands => "palette_recent_commands",
            Part::RecentSearches => "palette_recent_searches",
            Part::Commands => "palette_commands",
            Part::Tracks => "tracks",
            Part::Albums => "albums",
            Part::Artists => "artists",
            Part::Playlists => "playlists",
            Part::Navigate => "palette_navigate",
            Part::Settings => "settings",
        }
    }
}

/// Which blocks a pass draws, in the order they are drawn.
pub fn order(mode: Mode, searching: bool, current: bool) -> Vec<Part> {
    let mut parts = Vec::new();
    if current && matches!(mode, Mode::Everything | Mode::Commands) {
        parts.push(Part::CurrentTrack);
    }
    match mode {
        Mode::Commands => parts.push(Part::Commands),
        Mode::Artists => parts.push(Part::Artists),
        Mode::Albums => parts.push(Part::Albums),
        Mode::Playlists => parts.push(Part::Playlists),
        Mode::Everything if !searching => {
            parts.push(Part::RecentCommands);
            parts.push(Part::RecentSearches);
            parts.push(Part::Navigate);
            parts.push(Part::Commands);
        }
        Mode::Everything => {
            parts.push(Part::Commands);
            parts.push(Part::Tracks);
            parts.push(Part::Albums);
            parts.push(Part::Artists);
            parts.push(Part::Playlists);
            parts.push(Part::Navigate);
            parts.push(Part::Settings);
        }
    }
    parts
}

/// The next row the arrows land on, wrapping at both ends.
pub fn step(selected: usize, count: usize, forward: bool) -> usize {
    if count == 0 {
        return 0;
    }
    match forward {
        true => (selected + 1) % count,
        false => (selected + count - 1) % count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_picks_a_mode_and_is_not_part_of_the_query() {
        assert_eq!(parse(">play"), (Mode::Commands, "play"));
        assert_eq!(parse("@ portishead"), (Mode::Artists, "portishead"));
        assert_eq!(parse("#dummy"), (Mode::Albums, "dummy"));
        assert_eq!(parse(":road trip"), (Mode::Playlists, "road trip"));
    }

    #[test]
    fn plain_text_is_everything() {
        assert_eq!(parse("  glory box "), (Mode::Everything, "glory box"));
        assert_eq!(parse(""), (Mode::Everything, ""));
    }

    #[test]
    fn a_prefix_on_its_own_is_the_mode_with_nothing_typed() {
        assert_eq!(parse(">"), (Mode::Commands, ""));
    }

    #[test]
    fn the_current_track_leads_and_settings_come_last() {
        let parts = order(Mode::Everything, true, true);
        assert_eq!(parts.first(), Some(&Part::CurrentTrack));
        assert_eq!(parts.last(), Some(&Part::Settings));
        let navigate = parts.iter().position(|part| *part == Part::Navigate);
        let settings = parts.iter().position(|part| *part == Part::Settings);
        assert!(navigate < settings);
    }

    #[test]
    fn matches_come_between_the_current_track_and_navigate() {
        let parts = order(Mode::Everything, true, true);
        let current = parts.iter().position(|part| *part == Part::CurrentTrack);
        let tracks = parts.iter().position(|part| *part == Part::Tracks);
        let navigate = parts.iter().position(|part| *part == Part::Navigate);
        assert!(current < tracks && tracks < navigate);
    }

    #[test]
    fn nothing_playing_drops_the_current_track_block() {
        let parts = order(Mode::Everything, true, false);
        assert!(!parts.contains(&Part::CurrentTrack));
    }

    #[test]
    fn an_empty_box_offers_what_was_used_before() {
        let parts = order(Mode::Everything, false, false);
        assert_eq!(parts.first(), Some(&Part::RecentCommands));
        assert!(parts.contains(&Part::RecentSearches));
        assert!(!parts.contains(&Part::Tracks));
    }

    #[test]
    fn a_mode_draws_only_its_own_block() {
        assert_eq!(order(Mode::Albums, true, true), vec![Part::Albums]);
        assert_eq!(order(Mode::Artists, true, false), vec![Part::Artists]);
        assert_eq!(
            order(Mode::Commands, true, true),
            vec![Part::CurrentTrack, Part::Commands]
        );
    }

    #[test]
    fn the_arrows_wrap_at_both_ends() {
        assert_eq!(step(0, 3, false), 2);
        assert_eq!(step(2, 3, true), 0);
        assert_eq!(step(0, 3, true), 1);
        assert_eq!(step(2, 3, false), 1);
    }

    #[test]
    fn an_empty_list_has_nowhere_to_go() {
        assert_eq!(step(0, 0, true), 0);
        assert_eq!(step(4, 0, false), 0);
    }
}

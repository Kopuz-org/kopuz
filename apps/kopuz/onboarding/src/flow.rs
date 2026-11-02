//! The guide's rules, apart from GPUI so they can be tested: its steps, its groups and its language.

use api::schema::FieldKind;
use api::{JobKind, JobState, ServiceInfo};

/// Pick, Connect and Scan are one rail step, Music, drawn as the stages of a single section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Welcome,
    Pick,
    Connect,
    Scan,
    Appearance,
    Playback,
    Integrations,
    Privacy,
    Ready,
}

/// The rail's labels, one per numbered step.
pub const RAIL: [&str; 6] = [
    "language",
    "music",
    "appearance",
    "settings_group_playback",
    "settings_group_services",
    "onboarding_privacy_title",
];

impl Step {
    /// Where the step sits on the rail, from zero; the ready card sits outside it.
    pub fn index(self) -> Option<usize> {
        match self {
            Step::Welcome => Some(0),
            Step::Pick | Step::Connect | Step::Scan => Some(1),
            Step::Appearance => Some(2),
            Step::Playback => Some(3),
            Step::Integrations => Some(4),
            Step::Privacy => Some(5),
            Step::Ready => None,
        }
    }

    /// The step Back leads to; leaving Music backwards lands on its source page only if a source was added.
    pub fn back(self, added: bool) -> Option<Step> {
        match self {
            Step::Welcome => None,
            Step::Pick => Some(Step::Welcome),
            Step::Connect => Some(Step::Pick),
            Step::Scan => Some(Step::Connect),
            Step::Appearance if added => Some(Step::Scan),
            Step::Appearance => Some(Step::Pick),
            Step::Playback => Some(Step::Appearance),
            Step::Integrations => Some(Step::Playback),
            Step::Privacy => Some(Step::Integrations),
            Step::Ready => Some(Step::Privacy),
        }
    }

    /// Leaving this step backwards takes away the source it was filling.
    pub fn undoes_source(self) -> bool {
        self == Step::Scan
    }

    /// The source is connected by now, so its fill is still worth following even off the Scan stage.
    pub fn past_connect(self) -> bool {
        !matches!(self, Step::Welcome | Step::Pick | Step::Connect)
    }

    pub fn key(self) -> &'static str {
        match self {
            Step::Welcome => "welcome",
            Step::Pick => "pick",
            Step::Connect => "connect",
            Step::Scan => "scan",
            Step::Appearance => "appearance",
            Step::Playback => "playback",
            Step::Integrations => "integrations",
            Step::Privacy => "privacy",
            Step::Ready => "ready",
        }
    }
}

/// Where a service's card is listed, read off the form it publishes rather than its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Local,
    Server,
    Streaming,
}

impl Group {
    pub const ALL: [Group; 3] = [Group::Local, Group::Server, Group::Streaming];

    pub fn heading(self) -> &'static str {
        match self {
            Group::Local => "onboarding_group_local",
            Group::Server => "first_run_server",
            Group::Streaming => "first_run_streaming",
        }
    }

    pub fn hint(self) -> Option<&'static str> {
        match self {
            Group::Local => None,
            Group::Server => Some("first_run_server_hint"),
            Group::Streaming => Some("first_run_streaming_hint"),
        }
    }
}

/// Folders make it local, an address makes it a server, and anything else is signed into.
pub fn group_of(service: &ServiceInfo) -> Group {
    let has =
        |wanted: fn(&FieldKind) -> bool| service.fields.iter().any(|field| wanted(&field.kind));
    if has(|kind| matches!(kind, FieldKind::Directories)) {
        Group::Local
    } else if has(|kind| matches!(kind, FieldKind::Url)) {
        Group::Server
    } else {
        Group::Streaming
    }
}

/// The services in their groups, in the order the daemon listed them within each.
pub fn grouped(services: &[ServiceInfo]) -> Vec<(Group, Vec<ServiceInfo>)> {
    Group::ALL
        .iter()
        .map(|group| {
            let members: Vec<ServiceInfo> = services
                .iter()
                .filter(|service| group_of(service) == *group)
                .cloned()
                .collect();
            (*group, members)
        })
        .filter(|(_, members)| !members.is_empty())
        .collect()
}

/// The locale the session asks for, by POSIX precedence.
pub fn session_locale() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.trim().is_empty())
}

/// The Kopuz language a POSIX locale such as `pt_BR.UTF-8` names, among those `available`.
pub fn language_for(locale: &str, available: &[&str]) -> Option<String> {
    let bare = locale.split(['.', '@']).next()?.trim();
    if bare.is_empty() || bare == "C" || bare == "POSIX" {
        return None;
    }
    let mut parts = bare.split(['_', '-']);
    let language = parts.next()?.to_ascii_lowercase();
    let region = parts.next().map(str::to_ascii_uppercase);
    // Greek ships as `gr`, which is a country code, not a language one.
    let language = match language.as_str() {
        "el" => "gr".to_string(),
        _ => language,
    };
    let find = |code: &str| {
        available
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(code))
            .map(|found| found.to_string())
    };
    if let Some(region) = &region
        && let Some(found) = find(&format!("{language}-{region}"))
    {
        return Some(found);
    }
    if let Some(found) = find(&language) {
        return Some(found);
    }
    // A bare language with only regional builds, such as `pt` or `vi`, takes the first of them.
    available
        .iter()
        .find(|candidate| {
            candidate
                .split('-')
                .next()
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&language))
                && region.is_none()
        })
        .map(|found| found.to_string())
}

/// The guide opened again since this state was built, so none of it applies any more.
pub fn starts_over(held: u64, current: u64) -> bool {
    held != current
}

/// How the window's background shows the playing cover, as the guide offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
    Off,
    Cover,
    Blurred,
}

/// The blur "Blurred" sets: enough that the cover reads as colour, not as a picture.
pub const BACKDROP_BLUR: u8 = 40;

impl Backdrop {
    pub const ALL: [Backdrop; 3] = [Backdrop::Off, Backdrop::Cover, Backdrop::Blurred];

    pub fn of(background: &config::Background) -> Self {
        match (background.cover_art_background, background.cover_art_blur) {
            (false, _) => Backdrop::Off,
            (true, 0) => Backdrop::Cover,
            (true, _) => Backdrop::Blurred,
        }
    }

    /// The settings that make this the backdrop; a blur already chosen in Settings is kept.
    pub fn edits(self, background: &config::Background) -> Vec<config::BackgroundEdit> {
        use config::BackgroundEdit::{CoverArtBackground, CoverArtBlur};
        match self {
            Backdrop::Off => vec![CoverArtBackground(false)],
            Backdrop::Cover => vec![CoverArtBackground(true), CoverArtBlur(0)],
            Backdrop::Blurred => {
                let blur = match background.cover_art_blur {
                    0 => BACKDROP_BLUR,
                    kept => kept,
                };
                vec![CoverArtBackground(true), CoverArtBlur(blur)]
            }
        }
    }
}

/// A fresh install starts on the blurred cover; a background anyone already set is left alone.
pub fn starts_blurred(background: &config::Background) -> bool {
    *background == config::Background::default()
}

/// The jobs that fill a library: a folder scan, or a sync source's library, playlists and likes together.
pub const FILL_KINDS: [JobKind; 4] = [
    JobKind::Scan,
    JobKind::LibrarySync,
    JobKind::PlaylistSync,
    JobKind::FavoritesSync,
];

/// What a source needs started to fill it; a streaming library arrives through its playlists and likes, not its library sync alone.
pub fn fill_kinds(scan_folders: bool, sync: bool) -> &'static [JobKind] {
    match (scan_folders, sync) {
        (true, _) => &FILL_KINDS[..1],
        (false, true) => &FILL_KINDS[1..],
        (false, false) => &[],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    NotStarted,
    Running,
    Ended { ok: bool },
}

/// Where the fill stands, from the states of its jobs: over only when none still runs.
pub fn fill(jobs: &[JobState]) -> Fill {
    if jobs.is_empty() {
        Fill::NotStarted
    } else if jobs.contains(&JobState::Running) {
        Fill::Running
    } else {
        Fill::Ended {
            ok: jobs.iter().all(|state| *state == JobState::Finished),
        }
    }
}

/// The number in a daemon job id such as `job-12`, which only ever grows.
pub fn job_number(id: &str) -> Option<u64> {
    id.rsplit('-').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::schema::FieldSpec;

    fn service(id: &str, kinds: &[FieldKind]) -> ServiceInfo {
        ServiceInfo {
            id: id.into(),
            fields: kinds
                .iter()
                .map(|kind| FieldSpec {
                    kind: kind.clone(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    const LANGUAGES: &[&str] = &[
        "en", "de", "tr", "pt-BR", "pt-PT", "zh-CN", "vi-VN", "gr", "tok",
    ];

    #[test]
    fn music_is_one_rail_step_and_the_rail_has_no_gap() {
        let steps = [
            Step::Welcome,
            Step::Pick,
            Step::Connect,
            Step::Scan,
            Step::Appearance,
            Step::Playback,
            Step::Integrations,
            Step::Privacy,
        ];
        let indices: Vec<usize> = steps.iter().filter_map(|step| step.index()).collect();
        assert_eq!(indices, [0, 1, 1, 1, 2, 3, 4, 5]);
        assert_eq!(RAIL.len(), 6);
        assert_eq!(Step::Ready.index(), None);
    }

    #[test]
    fn back_walks_one_step_and_only_the_scan_takes_the_source_away() {
        assert_eq!(Step::Welcome.back(false), None);
        assert_eq!(Step::Connect.back(false), Some(Step::Pick));
        assert_eq!(Step::Scan.back(true), Some(Step::Connect));
        assert_eq!(Step::Ready.back(true), Some(Step::Privacy));
        assert!(Step::Scan.undoes_source());
        assert!(!Step::Connect.undoes_source());
        assert!(!Step::Privacy.undoes_source());
    }

    #[test]
    fn back_from_appearance_skips_a_music_step_that_added_nothing() {
        assert_eq!(Step::Appearance.back(true), Some(Step::Scan));
        assert_eq!(Step::Appearance.back(false), Some(Step::Pick));
    }

    #[test]
    fn folders_come_first_then_servers_then_streaming() {
        let services = vec![
            service("yt", &[FieldKind::Radio { options: vec![] }]),
            service("jellyfin", &[FieldKind::Url]),
            service("folders", &[FieldKind::Directories]),
            service("smb", &[FieldKind::Url, FieldKind::Note]),
        ];
        let groups = grouped(&services);
        let order: Vec<(Group, Vec<&str>)> = groups
            .iter()
            .map(|(group, members)| {
                (
                    *group,
                    members.iter().map(|member| member.id.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            order,
            vec![
                (Group::Local, vec!["folders"]),
                (Group::Server, vec!["jellyfin", "smb"]),
                (Group::Streaming, vec!["yt"]),
            ]
        );
    }

    #[test]
    fn an_empty_group_is_left_out() {
        let groups = grouped(&[service("folders", &[FieldKind::Directories])]);
        assert_eq!(groups.len(), 1);
    }

    #[test]
    fn a_locale_names_its_language() {
        assert_eq!(language_for("tr_TR.UTF-8", LANGUAGES), Some("tr".into()));
        assert_eq!(language_for("de_DE@euro", LANGUAGES), Some("de".into()));
        assert_eq!(language_for("en_US.UTF-8", LANGUAGES), Some("en".into()));
    }

    #[test]
    fn a_region_picks_its_own_build() {
        assert_eq!(language_for("pt_PT.UTF-8", LANGUAGES), Some("pt-PT".into()));
        assert_eq!(language_for("pt_BR", LANGUAGES), Some("pt-BR".into()));
        assert_eq!(language_for("zh_CN.UTF-8", LANGUAGES), Some("zh-CN".into()));
        assert_eq!(language_for("vi", LANGUAGES), Some("vi-VN".into()));
    }

    #[test]
    fn greek_maps_to_the_code_kopuz_ships_it_under() {
        assert_eq!(language_for("el_GR.UTF-8", LANGUAGES), Some("gr".into()));
    }

    #[test]
    fn nothing_matches_the_c_locale_or_an_unshipped_language() {
        assert_eq!(language_for("C", LANGUAGES), None);
        assert_eq!(language_for("POSIX", LANGUAGES), None);
        assert_eq!(language_for("C.UTF-8", LANGUAGES), None);
        assert_eq!(language_for("", LANGUAGES), None);
        assert_eq!(language_for("fi_FI.UTF-8", LANGUAGES), None);
        assert_eq!(language_for("zh_TW.UTF-8", LANGUAGES), None);
    }

    #[test]
    fn a_guide_opened_again_starts_from_the_welcome() {
        assert!(!starts_over(3, 3));
        assert!(starts_over(3, 4));
        // Back from the scan is the only way back that takes the source away, so a reopen never resumes it.
        let mut step = Step::Privacy;
        let mut undone = Vec::new();
        while let Some(previous) = step.back(true) {
            if step.undoes_source() {
                undone.push(step);
            }
            step = previous;
        }
        assert_eq!(step, Step::Welcome);
        assert_eq!(undone, vec![Step::Scan]);
    }

    #[test]
    fn the_backdrop_reads_back_what_it_writes() {
        let mut background = config::Background::default();
        assert_eq!(Backdrop::of(&background), Backdrop::Off);
        for look in Backdrop::ALL {
            for edit in look.edits(&background) {
                match edit {
                    config::BackgroundEdit::CoverArtBackground(on) => {
                        background.cover_art_background = on
                    }
                    config::BackgroundEdit::CoverArtBlur(blur) => background.cover_art_blur = blur,
                    _ => {}
                }
            }
            assert_eq!(Backdrop::of(&background), look);
        }
    }

    #[test]
    fn blurred_keeps_a_blur_already_chosen() {
        let background = config::Background {
            cover_art_blur: 75,
            ..Default::default()
        };
        assert!(
            Backdrop::Blurred
                .edits(&background)
                .contains(&config::BackgroundEdit::CoverArtBlur(75))
        );
    }

    #[test]
    fn only_an_untouched_background_starts_blurred() {
        assert!(starts_blurred(&config::Background::default()));
        let chosen = config::Background {
            cover_art_darkening: 30,
            ..Default::default()
        };
        assert!(!starts_blurred(&chosen));
    }

    #[test]
    fn a_sync_source_fills_through_its_playlists_and_likes_too() {
        assert_eq!(fill_kinds(true, false), &[JobKind::Scan]);
        assert_eq!(
            fill_kinds(false, true),
            &[
                JobKind::LibrarySync,
                JobKind::PlaylistSync,
                JobKind::FavoritesSync
            ]
        );
        assert!(fill_kinds(false, false).is_empty());
    }

    #[test]
    fn the_fill_is_over_only_when_none_of_its_jobs_runs() {
        assert_eq!(fill(&[]), Fill::NotStarted);
        // The library sync ending first is what used to count a still-empty library.
        assert_eq!(
            fill(&[JobState::Finished, JobState::Running, JobState::Running]),
            Fill::Running
        );
        assert_eq!(
            fill(&[JobState::Finished, JobState::Finished]),
            Fill::Ended { ok: true }
        );
        assert_eq!(
            fill(&[JobState::Finished, JobState::Failed]),
            Fill::Ended { ok: false }
        );
    }

    #[test]
    fn job_ids_read_as_their_number() {
        assert_eq!(job_number("job-12"), Some(12));
        assert_eq!(job_number("nonsense"), None);
    }
}

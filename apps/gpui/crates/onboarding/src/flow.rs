//! The guide's rules, apart from GPUI so they can be tested: its steps, its groups and its language.

use api::ServiceInfo;
use api::schema::FieldKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Welcome,
    Pick,
    Connect,
    Scan,
    Privacy,
    Ready,
}

/// The steps the rail numbers; the welcome and the ready card sit outside it.
pub const RAIL: [Step; 4] = [Step::Pick, Step::Connect, Step::Scan, Step::Privacy];

impl Step {
    /// Where the step sits on the rail, from zero.
    pub fn index(self) -> Option<usize> {
        RAIL.iter().position(|step| *step == self)
    }

    /// The step Back leads to; the welcome has nowhere further back.
    pub fn back(self) -> Option<Step> {
        match self {
            Step::Welcome => None,
            Step::Pick => Some(Step::Welcome),
            Step::Connect => Some(Step::Pick),
            Step::Scan => Some(Step::Connect),
            Step::Privacy => Some(Step::Scan),
            Step::Ready => Some(Step::Privacy),
        }
    }

    /// Leaving this step backwards takes away the source it was filling.
    pub fn undoes_source(self) -> bool {
        self == Step::Scan
    }

    pub fn key(self) -> &'static str {
        match self {
            Step::Welcome => "welcome",
            Step::Pick => "pick",
            Step::Connect => "connect",
            Step::Scan => "scan",
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
    fn the_rail_numbers_every_step_on_it_without_a_gap() {
        assert_eq!(Step::Welcome.index(), None);
        assert_eq!(Step::Pick.index(), Some(0));
        assert_eq!(Step::Connect.index(), Some(1));
        assert_eq!(Step::Scan.index(), Some(2));
        assert_eq!(Step::Privacy.index(), Some(3));
        assert_eq!(Step::Ready.index(), None);
    }

    #[test]
    fn back_walks_one_step_and_only_the_scan_takes_the_source_away() {
        assert_eq!(Step::Welcome.back(), None);
        assert_eq!(Step::Connect.back(), Some(Step::Pick));
        assert_eq!(Step::Scan.back(), Some(Step::Connect));
        assert_eq!(Step::Ready.back(), Some(Step::Privacy));
        assert!(Step::Scan.undoes_source());
        assert!(!Step::Connect.undoes_source());
        assert!(!Step::Privacy.undoes_source());
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
        while let Some(previous) = step.back() {
            if step.undoes_source() {
                undone.push(step);
            }
            step = previous;
        }
        assert_eq!(step, Step::Welcome);
        assert_eq!(undone, vec![Step::Scan]);
    }

    #[test]
    fn job_ids_read_as_their_number() {
        assert_eq!(job_number("job-12"), Some(12));
        assert_eq!(job_number("nonsense"), None);
    }
}

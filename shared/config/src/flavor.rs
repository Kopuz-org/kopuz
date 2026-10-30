//! Which Kopuz this build is, fixed at build time by `KOPUZ_FLAVOR`, so Kopuz and Kopuz Nightly never share a socket, a data directory or an install.

/// The app a build belongs to; anything but `KOPUZ_FLAVOR=nightly` builds the stable one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Stable,
    Nightly,
}

const BUILT: Option<&str> = option_env!("KOPUZ_FLAVOR");

const _: () = assert!(
    match BUILT {
        None => true,
        Some(name) => same(name, "stable") || same(name, "nightly"),
    },
    "KOPUZ_FLAVOR must be stable or nightly"
);

/// `==` on strings, which a const fn cannot call yet.
const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

impl Flavor {
    pub const fn current() -> Self {
        match BUILT {
            Some(name) if same(name, "nightly") => Self::Nightly,
            _ => Self::Stable,
        }
    }

    /// The directory, socket and binary-suffix name: `kopuz`, `kopuz-nightly`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Stable => "kopuz",
            Self::Nightly => "kopuz-nightly",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Stable => "Kopuz",
            Self::Nightly => "Kopuz Nightly",
        }
    }

    pub const fn app_id(self) -> &'static str {
        match self {
            Self::Stable => "moe.kopuz.kopuz",
            Self::Nightly => "moe.kopuz.kopuz.nightly",
        }
    }

    /// A binary of this flavour: `kopuzd` stays `kopuzd`, or becomes `kopuzd-nightly`.
    pub fn binary(self, base: &str) -> String {
        match self {
            Self::Stable => base.to_string(),
            Self::Nightly => format!("{base}-nightly"),
        }
    }

    /// The per-user directories this flavour keeps its library, settings and caches in.
    pub fn project_dirs(self) -> Option<directories::ProjectDirs> {
        directories::ProjectDirs::from("moe", "kopuz", self.name())
    }
}

/// [`Flavor::project_dirs`] for this build.
pub fn project_dirs() -> Option<directories::ProjectDirs> {
    Flavor::current().project_dirs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flavours_never_share_a_name() {
        assert_ne!(Flavor::Stable.name(), Flavor::Nightly.name());
        assert_ne!(Flavor::Stable.app_id(), Flavor::Nightly.app_id());
        assert_eq!(Flavor::Stable.binary("kopuzd"), "kopuzd");
        assert_eq!(Flavor::Nightly.binary("kopuzd"), "kopuzd-nightly");
    }

    #[test]
    fn a_plain_build_is_the_stable_app() {
        if option_env!("KOPUZ_FLAVOR").is_none() {
            assert_eq!(Flavor::current(), Flavor::Stable);
        }
    }
}

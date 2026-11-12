//! The command line, which only says how the app starts.

use std::ffi::OsString;

/// What the command line asked for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// Enter offline mode once the daemon answers.
    pub offline: bool,
    /// A `kopuz://` link the OS opened this app with.
    pub link: Option<String>,
}

impl Args {
    /// Read the arguments after the program name; an unknown one is said and skipped, not fatal.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Self {
        let mut parsed = Self::default();
        for arg in args {
            match arg.to_str() {
                Some("--offline") => parsed.offline = true,
                Some(link) if is_link(link) => parsed.link = Some(link.to_string()),
                _ => tracing::warn!(?arg, "unknown argument; ignored"),
            }
        }
        parsed
    }
}

/// A URL this app registers to open.
pub fn is_link(arg: &str) -> bool {
    arg.get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("kopuz://"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Args {
        Args::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn offline_is_asked_for_only_by_its_flag() {
        assert_eq!(parse(&[]), Args::default());
        assert!(parse(&["--offline"]).offline);
        assert!(parse(&["--verbose", "--offline"]).offline);
        assert!(!parse(&["--offline=no", "offline"]).offline);
    }

    #[test]
    fn a_kopuz_link_is_kept_and_any_other_url_is_not() {
        let link = "kopuz://j/1/ytmusic/p0/abc";
        assert_eq!(parse(&[link]).link.as_deref(), Some(link));
        assert_eq!(parse(&["KOPUZ://j/x"]).link.as_deref(), Some("KOPUZ://j/x"));
        assert_eq!(parse(&["https://kopuz.moe/j#1"]).link, None);
        assert_eq!(parse(&["kopuz:"]).link, None);
    }
}

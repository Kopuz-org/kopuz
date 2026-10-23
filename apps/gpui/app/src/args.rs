//! The command line, which only says how the app starts.

use std::ffi::OsString;

/// What the command line asked for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// Enter offline mode once the daemon answers.
    pub offline: bool,
}

impl Args {
    /// Read the arguments after the program name; an unknown one is said and skipped, not fatal.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Self {
        let mut parsed = Self::default();
        for arg in args {
            match arg.to_str() {
                Some("--offline") => parsed.offline = true,
                _ => tracing::warn!(?arg, "unknown argument; ignored"),
            }
        }
        parsed
    }
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
}

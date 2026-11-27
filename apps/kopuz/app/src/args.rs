//! The command line, which only says how the app starts.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// What the command line asked for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// Enter offline mode once the daemon answers.
    pub offline: bool,
    /// A `kopuz://` link the OS opened this app with.
    pub link: Option<String>,
    /// Local files the OS or command line asked to play.
    pub files: Vec<String>,
}

impl Args {
    /// Read the arguments after the program name; an unknown one is said and skipped, not fatal.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Self {
        let cwd = std::env::current_dir().ok();
        let mut parsed = Self::default();
        let mut literal = false;
        for arg in args {
            if !literal && arg == "--" {
                literal = true;
                continue;
            }
            if !literal && arg == "--offline" {
                parsed.offline = true;
                continue;
            }
            if !literal && let Some(text) = arg.to_str() {
                if is_link(text) {
                    parsed.link = Some(text.to_string());
                    continue;
                }
                if let Some(path) = file_url_path(text) {
                    parsed.files.push(path);
                    continue;
                }
            }
            if (literal || !arg.to_string_lossy().starts_with('-'))
                && let Some(path) = existing_file_path(Path::new(&arg), cwd.as_deref())
            {
                parsed.files.push(path);
                continue;
            }
            tracing::warn!(?arg, "unknown argument; ignored");
        }
        parsed
    }
}

/// A URL this app registers to open.
pub fn is_link(arg: &str) -> bool {
    arg.get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("kopuz://"))
}

/// Decode a `file://` URL into a local path string, canonicalizing when the file exists.
pub fn file_url_path(arg: &str) -> Option<String> {
    if !arg
        .get(..7)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file://"))
    {
        return None;
    }
    let url = url::Url::parse(arg).ok()?;
    let path = url.to_file_path().ok()?;
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    path.to_str().map(str::to_string)
}

/// Encode an absolute path as a `file://` URL so line-delimited relays preserve spaces and newlines.
pub fn file_url(path: &str) -> Option<String> {
    url::Url::from_file_path(path).ok().map(String::from)
}

/// Resolve `path` against `cwd` so the daemon does not depend on this process's working directory.
fn existing_file_path(path: &Path, cwd: Option<&Path>) -> Option<String> {
    let absolute: PathBuf = match (path.is_absolute(), cwd) {
        (true, _) => path.to_path_buf(),
        (false, Some(cwd)) => cwd.join(path),
        (false, None) => return None,
    };
    if !absolute.is_file() {
        return None;
    }
    let resolved = std::fs::canonicalize(&absolute).unwrap_or(absolute);
    resolved.to_str().map(str::to_string)
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

    #[test]
    fn file_urls_and_existing_paths_are_collected_in_order() {
        let dir = std::env::temp_dir().join(format!("kopuz-args-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let song_a = dir.join("01 track.flac");
        let song_b = dir.join("-02 dash.mp3");
        std::fs::write(&song_a, b"").unwrap();
        std::fs::write(&song_b, b"").unwrap();
        let canonical_a = std::fs::canonicalize(&song_a).unwrap();
        let canonical_b = std::fs::canonicalize(&song_b).unwrap();

        let uri = url::Url::from_file_path(&song_a).unwrap().to_string();
        let parsed = parse(&[
            &uri,
            song_a.to_str().unwrap(),
            "--",
            song_b.to_str().unwrap(),
        ]);
        assert_eq!(
            parsed.files,
            vec![
                canonical_a.to_str().unwrap().to_string(),
                canonical_a.to_str().unwrap().to_string(),
                canonical_b.to_str().unwrap().to_string(),
            ]
        );
        let missing = dir.join("missing song.mp3");
        let missing_uri = url::Url::from_file_path(&missing).unwrap().to_string();
        assert!(missing_uri.contains("%20"));
        assert_eq!(file_url_path(&missing_uri).as_deref(), missing.to_str());
        assert_eq!(file_url_path("https://example.com/song.mp3"), None);
        let roundtrip = file_url(canonical_a.to_str().unwrap()).unwrap();
        assert_eq!(file_url_path(&roundtrip).as_deref(), canonical_a.to_str());

        let _ = std::fs::remove_dir_all(&dir);
    }
}

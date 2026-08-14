//! Keeping credentials out of what a URL says when it is logged or shown.

/// `url` with any userinfo replaced by a placeholder. A Nextcloud stream URL
/// carries the app password there, so the raw string must never reach a log
/// line, an error message, or the UI.
pub fn redact_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let authority_start = scheme_end + 3;
    let authority_end = url[authority_start..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |i| authority_start + i);
    let authority = &url[authority_start..authority_end];

    let Some(at) = authority.rfind('@') else {
        return url.to_string();
    };

    format!(
        "{}***@{}",
        &url[..authority_start],
        &url[authority_start + at + 1..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_userinfo_and_leaves_the_rest() {
        assert_eq!(
            redact_url(
                "https://user:app-password@cloud.example.com/remote.php/dav/files/user/a.mp3"
            ),
            "https://***@cloud.example.com/remote.php/dav/files/user/a.mp3"
        );
    }

    #[test]
    fn keeps_urls_without_credentials() {
        assert_eq!(
            redact_url("https://cloud.example.com/a.mp3?x=1"),
            "https://cloud.example.com/a.mp3?x=1"
        );
        assert_eq!(redact_url("not a url"), "not a url");
    }

    #[test]
    fn an_at_in_the_path_is_not_userinfo() {
        assert_eq!(
            redact_url("https://host/music/me@example.com/a.mp3"),
            "https://host/music/me@example.com/a.mp3"
        );
    }
}

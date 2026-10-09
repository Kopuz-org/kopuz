//! Cookie reader for the isolated YT Music profile: turns the cookies read by
//! [`crate::cookies`] into the `Cookie:` header YT Music expects, requiring
//! the 1P auth cookies to be present.

use std::path::Path;

use config::Browser;

use crate::cookies::Cookie;

pub(crate) const DOMAIN: &str = "youtube.com";

/// The launch flags the isolated sign-in profile is made with, which a later
/// headless read of it has to repeat to decrypt it.
fn profile_args() -> Vec<String> {
    vec!["--password-store=basic".to_string()]
}

#[tracing::instrument(name = "yt.cookies_extract", skip(profile_root), fields(browser = %browser))]
pub async fn extract_from(browser: Browser, profile_root: &Path) -> Result<String, String> {
    let cookies =
        crate::cookies::read_profile_cookies(browser, profile_root, DOMAIN, &profile_args())
            .await?;
    let header = header(&cookies);
    if !has_auth(&header) {
        return Err(format!(
            "no auth cookies found in {} profile — sign in to YouTube Music there first",
            browser.label()
        ));
    }
    Ok(header)
}

/// The cookies a browser would send to music.youtube.com, as a `Cookie:`
/// header. A profile in daily use also holds large cookies for other YouTube
/// hosts, which push the header past what YouTube accepts. A name set on both
/// `.youtube.com` and a subdomain keeps the `.youtube.com` value, which is the
/// one every YouTube host receives.
pub(crate) fn header(cookies: &[Cookie]) -> String {
    let mut sent: Vec<&Cookie> = cookies
        .iter()
        .filter(|c| sent_to_music(&c.domain) && header_safe(&c.name) && header_safe(&c.value))
        .collect();
    sent.sort_by_key(|c| c.domain != ".youtube.com");
    let mut kept: Vec<&Cookie> = Vec::new();
    for cookie in sent {
        if !kept.iter().any(|k| k.name == cookie.name) {
            kept.push(cookie);
        }
    }
    kept.iter()
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ")
}

fn sent_to_music(domain: &str) -> bool {
    let host = domain.strip_prefix('.').unwrap_or(domain);
    host == "youtube.com" || host == "music.youtube.com"
}

fn has_auth(header: &str) -> bool {
    header.split(';').any(|p| {
        let Some((k, _)) = p.trim().split_once('=') else {
            return false;
        };
        k == "SAPISID" || k == "__Secure-3PAPISID"
    })
}

fn header_safe(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| (0x20..0x7f).contains(&b) && b != b';' && b != b',')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(domain: &str, name: &str, value: &str) -> Cookie {
        Cookie {
            domain: domain.to_string(),
            name: name.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn the_header_leaves_out_what_cannot_be_sent() {
        let cookies = [
            cookie(".youtube.com", "SAPISID", "a/b"),
            cookie(".youtube.com", "EMPTY", ""),
            cookie(".youtube.com", "LIST", "a,b"),
            cookie(".youtube.com", "SEMI", "a;b"),
            cookie("youtube.com", "PREF", "f6=40000000&tz=Europe.Lisbon"),
        ];
        let header = header(&cookies);
        assert_eq!(header, "SAPISID=a/b; PREF=f6=40000000&tz=Europe.Lisbon");
        assert!(has_auth(&header));
        assert!(!has_auth("VISITOR_INFO1_LIVE=v; YSC=y"));
    }

    #[test]
    fn the_header_holds_only_what_music_youtube_com_receives() {
        let cookies = [
            cookie("music.youtube.com", "SID", "sub"),
            cookie(".youtube.com", "SID", "s"),
            cookie(".youtube.com", "SAPISID", "a"),
            cookie("www.youtube.com", "ST-1x", "big"),
            cookie(".studio.youtube.com", "S", "x"),
            cookie("music.youtube.com", "LAST_RESULT_ENTRY_KEY", "k"),
        ];
        assert_eq!(
            header(&cookies),
            "SID=s; SAPISID=a; LAST_RESULT_ENTRY_KEY=k"
        );
    }
}

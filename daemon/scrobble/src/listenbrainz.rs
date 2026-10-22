//! The ListenBrainz account side: checking a user token, and finding one in a
//! browser session signed in to listenbrainz.org, whose settings page is where
//! ListenBrainz shows a signed-in user their token.

use std::time::Duration;

use reqwest::Client;

const SETTINGS: &str = "https://listenbrainz.org/settings/";
const VALIDATE: &str = "https://api.listenbrainz.org/1/validate-token";
const TIMEOUT: Duration = Duration::from_secs(15);

fn client() -> Client {
    utils::http::builder()
        .timeout(TIMEOUT)
        .build()
        .unwrap_or_else(|_| utils::http::client())
}

/// The user the token belongs to, or `None` for a token ListenBrainz does not
/// know.
pub async fn validate_token(token: &str) -> Result<Option<String>, reqwest::Error> {
    let json: serde_json::Value = client()
        .get(VALIDATE)
        .header("Authorization", crate::musicbrainz::auth_header(token))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(json["valid"]
        .as_bool()
        .unwrap_or(false)
        .then(|| json["user_name"].as_str().unwrap_or_default().to_string()))
}

/// The listenbrainz.org cookies as a `Cookie` header, once they hold a
/// session.
pub fn session_header<'a>(cookies: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<String> {
    let usable: Vec<(&str, &str)> = cookies
        .into_iter()
        .filter(|(name, value)| header_safe(name) && header_safe(value))
        .collect();
    usable
        .iter()
        .any(|(name, _)| *name == "session" || *name == "remember_token")
        .then(|| {
            usable
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ")
        })
}

fn header_safe(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| (0x20..0x7f).contains(&b) && b != b';' && b != b',')
}

/// The token the settings page shows for the session in `cookie_header`.
pub async fn token_from_session(cookie_header: &str) -> Option<String> {
    let http = client();
    // The settings page is a single-page app: a GET serves the shell and the
    // page posts to the same URL for its data. Either may carry the token.
    for request in [http.post(SETTINGS), http.get(SETTINGS)] {
        let response = request
            .header("Cookie", cookie_header)
            .header("Accept", "application/json, text/html")
            .send()
            .await;
        let body = match response {
            Ok(response) => response.text().await,
            Err(error) => Err(error),
        };
        match body {
            Ok(body) => {
                if let Some(token) = find_token(&body) {
                    return Some(token);
                }
            }
            Err(error) => tracing::debug!(%error, "listenbrainz settings page"),
        }
    }
    None
}

/// The UUID after an `auth_token` key, in JSON or in HTML-escaped JSON.
pub fn find_token(body: &str) -> Option<String> {
    let at = body.find("auth_token")?;
    let rest = &body[at + "auth_token".len()..];
    let window = &rest[..rest.len().min(64)];
    window
        .split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
        .find(|run| is_uuid(run))
        .map(str::to_string)
}

fn is_uuid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5
        && parts.iter().map(|p| p.len()).eq([8, 4, 4, 4, 12])
        && parts
            .iter()
            .all(|p| p.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "8f1c2a3b-4d5e-4f60-8a9b-0c1d2e3f4a5b";

    #[test]
    fn needs_a_session_cookie() {
        assert_eq!(session_header([("_ga", "x")]), None);
        assert_eq!(session_header([("session", "")]), None);
        assert_eq!(
            session_header([("session", "abc"), ("_ga", "x"), ("bad", "a;b")]),
            Some("session=abc; _ga=x".into())
        );
        assert_eq!(
            session_header([("remember_token", "1|abc")]),
            Some("remember_token=1|abc".into())
        );
    }

    #[test]
    fn finds_the_token_in_the_settings_page_data() {
        let json = format!(r#"{{"current_user":{{"auth_token":"{TOKEN}","name":"a"}}}}"#);
        assert_eq!(find_token(&json), Some(TOKEN.into()));
    }

    #[test]
    fn finds_the_token_in_the_settings_page_html() {
        let html = format!(
            r#"<html><body><div id="react-container"></div>
            <script type="application/json" id="page-react-props">
            {{&#34;user&#34;: {{&#34;name&#34;: &#34;someone&#34;, &#34;auth_token&#34;: &#34;{TOKEN}&#34;}}}}
            </script></body></html>"#
        );
        assert_eq!(find_token(&html), Some(TOKEN.into()));
    }

    #[test]
    fn a_signed_out_page_has_no_token() {
        assert_eq!(find_token(r#"{"auth_token":null}"#), None);
        assert_eq!(find_token(r#"{"auth_token":"not-a-uuid"}"#), None);
        assert_eq!(find_token("<html>Sign in</html>"), None);
    }
}

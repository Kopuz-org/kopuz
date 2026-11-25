//! A sign-in the person finishes in their browser, over the host's `browser`
//! import. It answers only while [`Source::login`](crate::Source::login) runs
//! for a service whose manifest says `sign-in = "browser"`.

use crate::bindings::kopuz::plugin::browser as wit;

pub use wit::SignInError as Error;

/// Plain text for a person; the generated `Display` is the debug form.
pub fn message(error: &Error) -> String {
    match error {
        Error::Unavailable(message) => format!("no browser sign-in: {message}"),
        Error::Denied(message) => format!("sign-in refused: {message}"),
        Error::Timeout => "the sign-in was not finished in time".to_string(),
        Error::Other(message) => message.clone(),
    }
}

/// What the service sent back with the redirect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    target: String,
}

impl Redirect {
    /// The path and query the browser was sent to.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// A query parameter, decoded.
    pub fn param(&self, name: &str) -> Option<String> {
        let (_, query) = self.target.split_once('?')?;
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    }
}

impl From<String> for Redirect {
    fn from(target: String) -> Self {
        Self { target }
    }
}

/// Open `authorize_url`, which must be on one of the manifest's hosts, and
/// wait for the browser to come back to `http://127.0.0.1:<port><path>`.
pub fn sign_in(authorize_url: &str, port: u16, path: &str) -> Result<Redirect, Error> {
    wit::sign_in(authorize_url, port, path).map(Redirect::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_redirect_reads_its_decoded_parameters() {
        let redirect = Redirect::from("/login?code=a%2Fb+c&state=xyz".to_string());
        assert_eq!(redirect.param("code").as_deref(), Some("a/b c"));
        assert_eq!(redirect.param("state").as_deref(), Some("xyz"));
        assert_eq!(redirect.param("error"), None);
        assert_eq!(Redirect::from("/login".to_string()).param("code"), None);
    }
}

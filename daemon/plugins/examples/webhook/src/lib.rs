//! An example integration: posts JSON to the address the person gives it.

use kopuz_plugin_sdk::SourceError;
use kopuz_plugin_sdk::http::Client;
use kopuz_plugin_sdk::integration::{Integration, Settings};

pub struct Webhook {
    http: Client,
    url: String,
    token: Option<String>,
}

impl Webhook {
    fn post(&self, body: &serde_json::Value) -> Result<(), SourceError> {
        let mut request = self.http.post(&self.url).json(body);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send()?;
        match response.is_success() {
            true => Ok(()),
            false => Err(SourceError::from_status(response.status(), "the webhook")),
        }
    }
}

impl Integration for Webhook {
    fn open(settings: Settings) -> Self {
        Webhook {
            http: Client::new(),
            url: settings.get("url").unwrap_or_default().to_string(),
            token: settings.get("token").map(str::to_string),
        }
    }

    fn validate(&self) -> Result<(), SourceError> {
        if self.url.is_empty() {
            return Err(SourceError::InvalidInput(
                "the webhook needs an address".into(),
            ));
        }
        self.post(&serde_json::json!({ "event": "hello" }))
    }
}

kopuz_plugin_sdk::export_integration!(Webhook);

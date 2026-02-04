use jellyfin_sdk_rust::JellyfinSDK;
use serde::{Deserialize, Serialize};

pub struct JellyfinRemote {
    client: JellyfinSDK,
    base_url: String,
    device_id: String,
}

#[derive(Serialize)]
struct LoginRequest<'a> {
    #[serde(rename = "Username")]
    username: &'a str,
    #[serde(rename = "Pw")]
    password: &'a str,
}

#[derive(Deserialize)]
struct LoginResponse {
    #[serde(rename = "AccessToken")]
    access_token: String,
    #[serde(rename = "User")]
    #[allow(dead_code)]
    user: UserObj,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct UserObj {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name")]
    name: String,
}

impl JellyfinRemote {
    pub fn new(base_url: &str, api_key: Option<&str>, device_id: &str) -> Self {
        let mut client = JellyfinSDK::new();
        let clean_base_url = base_url.trim_end_matches('/');
        client.create_api(clean_base_url, api_key);

        Self {
            client,
            base_url: clean_base_url.to_string(),
            device_id: device_id.to_string(),
        }
    }

    pub async fn login(&mut self, username: &str, password: &str) -> Result<String, String> {
        let url = format!("{}/Users/AuthenticateByName", self.base_url);

        let client = reqwest::Client::new();
        let body = LoginRequest { username, password };

        let auth_header = format!(
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.1.0\"",
            self.device_id
        );

        let resp = client
            .post(&url)
            .header("X-Emby-Authorization", auth_header)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Login failed with status: {} - {}", status, text));
        }

        let login_resp: LoginResponse = resp.json().await.map_err(|e| e.to_string())?;

        self.client
            .create_api(&self.base_url, Some(&login_resp.access_token));

        Ok(login_resp.access_token)
    }

    pub async fn get_library_items(&self) -> Result<Vec<String>, String> {
        unimplemented!()
    }
}

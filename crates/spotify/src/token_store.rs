//! Secure token storage abstraction.
//!
//! The default implementation uses the OS keyring via the `keyring` crate.
//! A development-only plaintext fallback is gated behind the
//! `insecure-token-cache` feature.

use crate::error::{Result, SpotifyError};
use crate::types::Tokens;

const SERVICE: &str = "kopuz.spotify";
const ACCOUNT: &str = "tokens";

#[async_trait::async_trait]
pub trait TokenStore: Send + Sync {
    async fn load(&self) -> Result<Option<Tokens>>;
    async fn save(&self, tokens: &Tokens) -> Result<()>;
    async fn clear(&self) -> Result<()>;
}

#[cfg(not(target_arch = "wasm32"))]
pub struct KeyringTokenStore {
    service: String,
    account: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl KeyringTokenStore {
    pub fn new() -> Self {
        Self {
            service: SERVICE.into(),
            account: ACCOUNT.into(),
        }
    }

    pub fn with_account(account: impl Into<String>) -> Self {
        Self {
            service: SERVICE.into(),
            account: account.into(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for KeyringTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
impl TokenStore for KeyringTokenStore {
    async fn load(&self) -> Result<Option<Tokens>> {
        let service = self.service.clone();
        let account = self.account.clone();
        let raw = tokio::task::spawn_blocking(move || -> Result<Option<String>> {
            let entry = keyring::Entry::new(&service, &account)
                .map_err(|e| SpotifyError::Io(format!("keyring entry: {e}")))?;
            match entry.get_password() {
                Ok(s) => Ok(Some(s)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(SpotifyError::Io(format!("keyring get: {e}"))),
            }
        })
        .await
        .map_err(|e| SpotifyError::Io(format!("join: {e}")))??;
        match raw {
            None => Ok(None),
            Some(s) => Ok(Some(serde_json::from_str(&s)?)),
        }
    }

    async fn save(&self, tokens: &Tokens) -> Result<()> {
        let service = self.service.clone();
        let account = self.account.clone();
        let payload = serde_json::to_string(tokens)?;
        tokio::task::spawn_blocking(move || -> Result<()> {
            let entry = keyring::Entry::new(&service, &account)
                .map_err(|e| SpotifyError::Io(format!("keyring entry: {e}")))?;
            entry
                .set_password(&payload)
                .map_err(|e| SpotifyError::Io(format!("keyring set: {e}")))
        })
        .await
        .map_err(|e| SpotifyError::Io(format!("join: {e}")))?
    }

    async fn clear(&self) -> Result<()> {
        let service = self.service.clone();
        let account = self.account.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let entry = keyring::Entry::new(&service, &account)
                .map_err(|e| SpotifyError::Io(format!("keyring entry: {e}")))?;
            match entry.delete_credential() {
                Ok(()) => Ok(()),
                Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(SpotifyError::Io(format!("keyring delete: {e}"))),
            }
        })
        .await
        .map_err(|e| SpotifyError::Io(format!("join: {e}")))?
    }
}

/// In-memory store for tests and the early auth handshake.
#[derive(Default)]
pub struct MemoryTokenStore {
    inner: tokio::sync::Mutex<Option<Tokens>>,
}

impl MemoryTokenStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl TokenStore for MemoryTokenStore {
    async fn load(&self) -> Result<Option<Tokens>> {
        Ok(self.inner.lock().await.clone())
    }
    async fn save(&self, tokens: &Tokens) -> Result<()> {
        *self.inner.lock().await = Some(tokens.clone());
        Ok(())
    }
    async fn clear(&self) -> Result<()> {
        *self.inner.lock().await = None;
        Ok(())
    }
}

#[cfg(feature = "insecure-token-cache")]
pub struct PlaintextFileTokenStore {
    path: std::path::PathBuf,
}

#[cfg(feature = "insecure-token-cache")]
impl PlaintextFileTokenStore {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[cfg(feature = "insecure-token-cache")]
#[async_trait::async_trait]
impl TokenStore for PlaintextFileTokenStore {
    async fn load(&self) -> Result<Option<Tokens>> {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(s) => Ok(Some(serde_json::from_str(&s)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(SpotifyError::Io(e.to_string())),
        }
    }
    async fn save(&self, tokens: &Tokens) -> Result<()> {
        let s = serde_json::to_string(tokens)?;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        tokio::fs::write(&self.path, s)
            .await
            .map_err(|e| SpotifyError::Io(e.to_string()))
    }
    async fn clear(&self) -> Result<()> {
        match tokio::fs::remove_file(&self.path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(SpotifyError::Io(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_store_roundtrip() {
        let s = MemoryTokenStore::new();
        assert!(s.load().await.unwrap().is_none());
        let t = Tokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: 100,
            scope: "x".into(),
            token_type: "Bearer".into(),
        };
        s.save(&t).await.unwrap();
        let got = s.load().await.unwrap().unwrap();
        assert_eq!(got.access_token, "a");
        s.clear().await.unwrap();
        assert!(s.load().await.unwrap().is_none());
    }
}

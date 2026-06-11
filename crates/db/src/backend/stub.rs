//! In-memory `Storage` stub for the wasm/web target (not a shipped target — no
//! persistence). Exists only so `dx build --platform web` compiles. Swap for a
//! `wa-sqlite` + OPFS backend if web ever ships (no call-site changes).

use std::sync::Mutex;

use crate::{DbError, Storage};

pub struct Stub {
    config: Mutex<Option<config::AppConfig>>,
}

impl Stub {
    pub fn new() -> Self {
        Self {
            config: Mutex::new(None),
        }
    }
}

#[async_trait::async_trait]
impl Storage for Stub {
    async fn load_config(&self) -> Result<Option<config::AppConfig>, DbError> {
        Ok(self.config.lock().unwrap().clone())
    }

    async fn save_config(&self, cfg: &config::AppConfig) -> Result<(), DbError> {
        *self.config.lock().unwrap() = Some(cfg.clone());
        Ok(())
    }

    async fn import_legacy_json(
        &self,
        _config_dir: &std::path::Path,
    ) -> Result<crate::ImportReport, DbError> {
        Ok(crate::ImportReport::default())
    }

    async fn finalize_migration(&self, _config_dir: &std::path::Path) -> Result<usize, DbError> {
        Ok(0)
    }
}

//! The configured sources: which one is active, the saved servers with the active one's credentials, and their library roots.

use std::collections::HashMap;

use config::{MusicServer, MusicService, SavedLocalSource, SavedServer, Source};

#[derive(Debug, Clone, PartialEq)]
pub struct Sources {
    pub active: Source,
    /// The active server hydrated with its credentials; `None` while a folder source is active.
    pub server: Option<MusicServer>,
    pub servers: Vec<SavedServer>,
    /// Library roots per server id, for backends whose library is a folder tree (Nextcloud); no entry means auto-detect.
    pub folders: HashMap<String, Vec<String>>,
    pub device_id: String,
}

fn new_device_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl Default for Sources {
    fn default() -> Self {
        Self {
            active: Source::default(),
            server: None,
            servers: Vec::new(),
            folders: HashMap::new(),
            device_id: new_device_id(),
        }
    }
}

impl From<db::StoredSources> for Sources {
    fn from(stored: db::StoredSources) -> Self {
        Self {
            active: stored.active,
            server: stored.server,
            servers: stored.servers,
            folders: stored.folders,
            device_id: stored.device_id.unwrap_or_else(new_device_id),
        }
    }
}

impl Sources {
    /// Library roots configured for a server, empty when it auto-detects.
    pub fn folders_for(&self, server_id: &str) -> Vec<String> {
        self.folders.get(server_id).cloned().unwrap_or_default()
    }

    /// Replace a server's library roots, dropping the entry when the list empties so the backend auto-detects again.
    pub fn set_folders_for(&mut self, server_id: &str, folders: Vec<String>) {
        if folders.is_empty() {
            self.folders.remove(server_id);
        } else {
            self.folders.insert(server_id.to_string(), folders);
        }
    }

    pub fn find_saved_server(&self, id: &str) -> Option<&SavedServer> {
        self.servers.iter().find(|saved| saved.id == id)
    }

    pub fn remove_saved_server(&mut self, id: &str) {
        self.servers.retain(|saved| saved.id != id);
        self.folders.remove(id);
        if self
            .server
            .as_ref()
            .is_some_and(|active| active.id.as_deref() == Some(id))
        {
            self.server = None;
        }
    }

    pub fn clear_active(&mut self) {
        self.active = Source::default();
        self.server = None;
    }

    pub fn set_active_local(&mut self, source: Source) {
        debug_assert!(source.is_local());
        self.active = source;
        self.server = None;
    }

    pub fn set_active_server(&mut self, server: MusicServer) {
        self.active = server.id.clone().map_or(Source::default(), Source::Server);
        self.server = Some(server);
    }

    pub fn active_service(&self) -> Option<MusicService> {
        self.active.server_id()?;
        self.server.as_ref().map(|server| server.service)
    }

    /// The active source's server id, falling back to the hydrated server's own (the legacy single-server shape).
    pub fn active_server_id(&self) -> Option<String> {
        self.active
            .server_id()
            .map(String::from)
            .or_else(|| self.server.as_ref().and_then(|server| server.id.clone()))
    }

    /// Whether `source` still exists, as opposed to one just deleted; folder sources are settings, so the caller passes them.
    pub fn has_source(&self, local_sources: &[SavedLocalSource], source: &Source) -> bool {
        match source {
            Source::LocalLibrary(id) => local_sources.iter().any(|saved| &saved.id == id),
            Source::Server(id) => self.servers.iter().any(|saved| &saved.id == id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_folders_round_trip_and_clear_when_emptied() {
        let mut sources = Sources::default();
        assert!(sources.folders_for("srv").is_empty());

        sources.set_folders_for("srv", vec!["/Music".to_string()]);
        assert_eq!(sources.folders_for("srv"), vec!["/Music".to_string()]);

        // Emptying drops the entry, so the backend auto-detects again.
        sources.set_folders_for("srv", Vec::new());
        assert!(sources.folders_for("srv").is_empty());
        assert!(!sources.folders.contains_key("srv"));
    }

    #[test]
    fn a_stored_device_id_is_kept_and_a_missing_one_is_made() {
        let kept = Sources::from(db::StoredSources {
            device_id: Some("dev-1".into()),
            ..Default::default()
        });
        assert_eq!(kept.device_id, "dev-1");
        let made = Sources::from(db::StoredSources::default());
        assert!(!made.device_id.is_empty());
    }
}

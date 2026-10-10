//! The AppUserModelID the daemon runs under. On Windows this is what the
//! media flyout and notifications name and badge the process by, so a
//! frontend that spawns kopuzd passes its own id to appear as itself there.
//! Without one the process sets none and Windows labels it by its exe.

use std::path::PathBuf;

/// The shell rejects longer ids.
const MAX_APP_ID_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIdentity {
    pub id: String,
    /// Shown for the id by the flyout, through the registry.
    pub name: Option<String>,
    /// An `.ico` or `.png` shown beside the name.
    pub icon: Option<PathBuf>,
}

/// Check an id before it reaches the shell. It doubles as a registry key
/// name, so a backslash would land the values under some other key.
pub fn validate_app_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("--app-id must not be empty".to_string());
    }
    if id.chars().count() > MAX_APP_ID_LEN {
        return Err(format!(
            "--app-id is longer than {MAX_APP_ID_LEN} characters"
        ));
    }
    if id.chars().any(|c| c.is_whitespace() || c == '\\') {
        return Err("--app-id must not contain spaces or backslashes".to_string());
    }
    Ok(())
}

impl AppIdentity {
    /// Where the shell looks up a display name and icon for an id that no
    /// Start menu shortcut carries, relative to `HKEY_CURRENT_USER`.
    pub fn registry_key(&self) -> String {
        format!(r"Software\Classes\AppUserModelId\{}", self.id)
    }

    /// The values to write under [`Self::registry_key`]; empty when there is
    /// nothing to register.
    pub fn registry_values(&self) -> Vec<(&'static str, String)> {
        let mut values = Vec::new();
        if let Some(name) = &self.name {
            values.push(("DisplayName", name.clone()));
        }
        if let Some(icon) = &self.icon {
            values.push(("IconUri", icon.display().to_string()));
        }
        values
    }

    /// Run the process under this id and register its name and icon. Must
    /// run before the SMTC window exists, since the shell reads the id when
    /// the media session is created.
    #[cfg(windows)]
    pub fn apply(&self) {
        let values = self.registry_values();
        if !values.is_empty() {
            let key = self.registry_key();
            let written = windows_registry::CURRENT_USER
                .create(&key)
                .and_then(|handle| {
                    values
                        .iter()
                        .try_for_each(|(name, value)| handle.set_string(name, value))
                });
            if let Err(error) = written {
                tracing::warn!(%error, key, "could not register the app identity");
            }
        }
        let id = windows::core::HSTRING::from(self.id.as_str());
        // SAFETY: the HSTRING outlives the call, which copies the string.
        let set =
            unsafe { windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(&id) };
        match set {
            Ok(()) => tracing::info!(id = %self.id, "app user model id set"),
            Err(error) => {
                tracing::warn!(%error, id = %self.id, "could not set the app user model id")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_id_has_nothing_to_register() {
        let identity = AppIdentity {
            id: "a.b".to_string(),
            name: None,
            icon: None,
        };
        assert!(identity.registry_values().is_empty());
    }

    #[test]
    fn registry_values_carry_name_and_icon() {
        let identity = AppIdentity {
            id: "es.canarycoders.formalmusic".to_string(),
            name: Some("FormalMusic".to_string()),
            icon: Some(PathBuf::from(r"C:\Apps\FormalMusic\formalmusic.ico")),
        };
        assert_eq!(
            identity.registry_key(),
            r"Software\Classes\AppUserModelId\es.canarycoders.formalmusic"
        );
        assert_eq!(
            identity.registry_values(),
            vec![
                ("DisplayName", "FormalMusic".to_string()),
                (
                    "IconUri",
                    r"C:\Apps\FormalMusic\formalmusic.ico".to_string()
                ),
            ]
        );
    }

    #[test]
    fn registry_values_skip_what_was_not_given() {
        let identity = AppIdentity {
            id: "a.b".to_string(),
            name: Some("Kopuz".to_string()),
            icon: None,
        };
        assert_eq!(
            identity.registry_values(),
            vec![("DisplayName", "Kopuz".to_string())]
        );
    }

    #[test]
    fn rejects_ids_the_shell_or_registry_would_misread() {
        assert!(validate_app_id("es.canarycoders.formalmusic").is_ok());
        assert!(validate_app_id("").is_err());
        assert!(validate_app_id("has space").is_err());
        assert!(validate_app_id(r"a\b").is_err());
        assert!(validate_app_id(&"a".repeat(129)).is_err());
        assert!(validate_app_id(&"a".repeat(128)).is_ok());
    }
}

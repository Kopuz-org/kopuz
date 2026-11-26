//! The integration plugins the person set up: each published as an
//! integration with the fields its manifest lists, and the ones turned on kept
//! running, one instance per plugin, until they are turned off, cleared or the
//! plugin goes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use api::{
    ApiError, ChoiceOption, ConnectKind, ErrorReason, FieldKind, FieldSpec, FieldValue, Icon,
    IntegrationInfo, Text,
};
use plugin_host::manifest::{self, Manifest};
use plugin_host::{Integration, Plugin, wit};

use crate::credentials::{CredentialStore, Credentials, PluginAnswers};

/// How an integration plugin's id is told apart from a built-in integration's.
pub const ID_PREFIX: &str = "plugin:";

/// The toggle the host puts above a plugin's own fields; a manifest cannot
/// name a field with a `$`.
const ENABLED: &str = "$enabled";

const FALLBACK_ICON: &str = "fa-solid fa-puzzle-piece";

/// The plugin an integration id names, if one is loaded.
pub fn plugin_of(id: &str) -> Option<Arc<Plugin>> {
    let plugin_id = id.strip_prefix(ID_PREFIX)?;
    loaded().into_iter().find(|plugin| plugin.id() == plugin_id)
}

/// Every integration plugin the daemon runs now, in the registry's order.
fn loaded() -> Vec<Arc<Plugin>> {
    server::plugins::registry()
        .plugins()
        .iter()
        .filter(|plugin| plugin.manifest().is_integration())
        .cloned()
        .collect()
}

fn spec(manifest: &Manifest) -> &manifest::Integration {
    static EMPTY: std::sync::OnceLock<manifest::Integration> = std::sync::OnceLock::new();
    manifest
        .integration
        .as_ref()
        .unwrap_or_else(|| EMPTY.get_or_init(Default::default))
}

fn enabled(answers: &PluginAnswers) -> bool {
    answers.get(ENABLED).is_some_and(|value| value == "true")
}

/// What the plugin is opened with: each field's answer, or its default.
fn settings(integration: &manifest::Integration, answers: &PluginAnswers) -> Vec<(String, String)> {
    integration
        .fields
        .iter()
        .map(|field| {
            let value = answers
                .get(&field.key)
                .cloned()
                .filter(|value| !value.is_empty())
                .or_else(|| match field.kind {
                    manifest::FieldKind::Secret => None,
                    _ => field.default.clone(),
                })
                .unwrap_or_else(|| match field.kind {
                    manifest::FieldKind::Toggle => "false".to_string(),
                    _ => String::new(),
                });
            (field.key.clone(), value)
        })
        .collect()
}

/// Whether every required field has something in it.
fn complete(integration: &manifest::Integration, settings: &[(String, String)]) -> bool {
    integration
        .fields
        .iter()
        .filter(|field| field.required)
        .all(|field| {
            settings
                .iter()
                .any(|(key, value)| *key == field.key && !value.trim().is_empty())
        })
}

/// Whether every required secret field has one stored. A frontend reads an
/// integration's `configured` as that, to show a blank secret box as kept; an
/// optional secret left empty does not keep it from reading as set up.
fn secrets_held(integration: &manifest::Integration, answers: &PluginAnswers) -> bool {
    integration
        .fields
        .iter()
        .filter(|field| field.kind == manifest::FieldKind::Secret && field.required)
        .all(|field| {
            answers
                .get(&field.key)
                .is_some_and(|value| !value.is_empty())
        })
}

/// Turned on, with every required field answered.
fn runs(integration: &manifest::Integration, answers: &PluginAnswers) -> bool {
    enabled(answers) && complete(integration, &settings(integration, answers))
}

fn kind_of(field: &manifest::Field) -> FieldKind {
    match field.kind {
        manifest::FieldKind::Text => FieldKind::Text,
        manifest::FieldKind::Secret => FieldKind::Secret,
        manifest::FieldKind::Url => FieldKind::Url,
        manifest::FieldKind::Toggle => FieldKind::Toggle,
        manifest::FieldKind::Choice => FieldKind::Choice {
            options: field
                .options
                .iter()
                .map(|option| ChoiceOption {
                    value: option.value.clone(),
                    label: Text::Literal(option.label.clone()),
                    unavailable: None,
                })
                .collect(),
            custom: false,
        },
    }
}

/// A plugin's integration as a settings page lists it. Its own words are
/// literals, in whatever language the plugin was written in.
fn info(plugin: &Plugin, credentials: &Credentials) -> IntegrationInfo {
    let manifest = plugin.manifest();
    let integration = spec(manifest);
    let answers = credentials.plugin(plugin.id());
    let current = settings(integration, &answers);
    // Named like Discord's presence toggle, so it reads right where a page shows no title above it.
    let mut fields = vec![FieldSpec {
        key: ENABLED.to_string(),
        label: Text::Literal(manifest.name.clone()),
        help: (!manifest.description.is_empty())
            .then(|| Text::Literal(manifest.description.clone())),
        kind: FieldKind::Toggle,
        value: Some(enabled(&answers).to_string()),
        ..Default::default()
    }];
    fields.extend(integration.fields.iter().map(|field| {
        let secret = field.kind == manifest::FieldKind::Secret;
        FieldSpec {
            key: field.key.clone(),
            label: Text::Literal(field.label.clone()),
            help: field.help.clone().map(Text::Literal),
            placeholder: field.placeholder.clone().map(Text::Literal),
            kind: kind_of(field),
            required: field.required,
            value: (!secret)
                .then(|| {
                    current
                        .iter()
                        .find(|(key, _)| *key == field.key)
                        .map(|(_, value)| value.clone())
                })
                .flatten(),
            ..Default::default()
        }
    }));
    IntegrationInfo {
        id: format!("{ID_PREFIX}{}", plugin.id()),
        name: Text::Literal(manifest.name.clone()),
        icon: integration
            .icon
            .clone()
            .map_or_else(|| Icon::Class(FALLBACK_ICON.into()), Icon::Svg),
        configured: runs(integration, &answers) && secrets_held(integration, &answers),
        connect: ConnectKind::None,
        fields,
    }
}

/// Every integration plugin loaded, as a settings page lists them.
pub fn infos(credentials: &Credentials) -> Vec<IntegrationInfo> {
    loaded()
        .iter()
        .map(|plugin| info(plugin, credentials))
        .collect()
}

pub fn info_of(id: &str, credentials: &Credentials) -> Option<IntegrationInfo> {
    plugin_of(id).map(|plugin| info(&plugin, credentials))
}

fn place_of(place: manifest::ActionPlace) -> api::ActionPlace {
    match place {
        manifest::ActionPlace::Palette => api::ActionPlace::Palette,
        manifest::ActionPlace::Track => api::ActionPlace::Track,
        manifest::ActionPlace::Album => api::ActionPlace::Album,
        manifest::ActionPlace::Artist => api::ActionPlace::Artist,
    }
}

/// The most of a panel a plugin can have drawn, so one that answers with too
/// much costs the page no more than this.
const MAX_BLOCKS: usize = 50;
const MAX_TEXT: usize = 2_000;
const MAX_TRACKS: usize = 100;

/// `text` cut to [`MAX_TEXT`] characters.
fn capped(text: String) -> String {
    match text.char_indices().nth(MAX_TEXT) {
        Some((end, _)) => text[..end].to_string(),
        None => text,
    }
}

fn block_of(block: wit::Block) -> api::PanelBlock {
    match block {
        wit::Block::Heading(text) => api::PanelBlock::Heading(capped(text)),
        wit::Block::Text(text) => api::PanelBlock::Text(capped(text)),
        wit::Block::Tracks(keys) => {
            api::PanelBlock::Tracks(keys.into_iter().take(MAX_TRACKS).collect())
        }
        wit::Block::Action(button) => api::PanelBlock::Action {
            label: capped(button.label),
            action: button.action,
        },
    }
}

/// Plain text for a person, since the generated `Display` is the debug form.
pub fn message(error: &wit::SourceError) -> String {
    match error {
        wit::SourceError::Unsupported(what) => format!("the plugin doesn't support {what}"),
        wit::SourceError::Connectivity => "the plugin could not reach its address".to_string(),
        wit::SourceError::Auth => "the address refused the credentials".to_string(),
        wit::SourceError::InvalidInput(message) | wit::SourceError::Backend(message) => {
            message.clone()
        }
    }
}

/// Why the plugin turned its answers down, with a reason a frontend translates where one fits.
fn rejected(error: &wit::SourceError) -> ApiError {
    let text = message(error);
    match error {
        wit::SourceError::Auth => {
            ApiError::invalid_input(text).because(ErrorReason::CredentialRejected)
        }
        wit::SourceError::Connectivity => {
            ApiError::internal(text).because(ErrorReason::IntegrationUnreachable)
        }
        _ => ApiError::invalid_input(text),
    }
}

/// Fold `values` into what is stored: a secret answered empty keeps the one
/// held, and a key the manifest does not list is refused.
fn merge(
    integration: &manifest::Integration,
    stored: PluginAnswers,
    values: &[FieldValue],
) -> Result<PluginAnswers, ApiError> {
    let mut answers = stored;
    for value in values {
        if value.key == ENABLED {
            answers.insert(ENABLED.to_string(), (value.value == "true").to_string());
            continue;
        }
        let Some(field) = integration.field(&value.key) else {
            return Err(ApiError::invalid_input(format!(
                "the integration has no field {}",
                value.key
            )));
        };
        let typed = value.value.trim();
        if field.kind == manifest::FieldKind::Secret && typed.is_empty() {
            continue;
        }
        answers.insert(field.key.clone(), typed.to_string());
    }
    Ok(answers)
}

/// The integration plugins turned on, each with one instance kept open.
pub struct PluginIntegrations {
    credentials: CredentialStore,
    running: Mutex<HashMap<String, Arc<Integration>>>,
    /// Held across an edit to the answers, from reading them to the running
    /// copy being swapped, so two edits at once cannot interleave and leave
    /// one's copy running on the other's answers.
    edits: tokio::sync::Mutex<()>,
    /// How an integration reaches kopuz, once the session exists.
    bridge: std::sync::RwLock<Option<Arc<dyn plugin_host::Bridge>>>,
}

impl PluginIntegrations {
    pub fn new(credentials: CredentialStore) -> Arc<Self> {
        Arc::new(Self {
            credentials,
            running: Mutex::new(HashMap::new()),
            edits: tokio::sync::Mutex::new(()),
            bridge: std::sync::RwLock::new(None),
        })
    }

    /// Let every integration opened from now on reach kopuz through `bridge`,
    /// as far as its manifest allows; the running ones reopen with it.
    pub fn attach_bridge(&self, bridge: Arc<dyn plugin_host::Bridge>) {
        *self.bridge.write().unwrap_or_else(PoisonError::into_inner) = Some(bridge);
        let retired: Vec<_> = self.lock().drain().map(|(_, live)| live).collect();
        retire(retired);
    }

    fn open(&self, plugin: Arc<Plugin>, settings: Vec<(String, String)>) -> Integration {
        let integration = Integration::new(plugin, settings);
        match self
            .bridge
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            Some(bridge) => integration.with_bridge(bridge),
            None => integration,
        }
    }

    /// Answer a plugin's fields. Once the answers leave it turned on with
    /// every required field filled, and change what it would run with, the
    /// plugin checks them first, and they are kept only if it accepts them.
    pub async fn set(&self, id: &str, values: &[FieldValue]) -> Result<IntegrationInfo, ApiError> {
        let _edit = self.edits.lock().await;
        let plugin = plugin_of(id).ok_or_else(|| ApiError::not_found("no such integration"))?;
        let integration = spec(plugin.manifest());
        let stored = self.credentials.snapshot().plugin(plugin.id());
        let answers = merge(integration, stored.clone(), values)?;
        let wanted = settings(integration, &answers);
        let unchanged = runs(integration, &stored) && settings(integration, &stored) == wanted;
        let mut checked = None;
        if runs(integration, &answers) && !unchanged {
            // A hub that takes one client at a time is busy while the copy on the old answers runs.
            let old = self.lock().remove(plugin.id());
            if let Some(old) = old {
                old.close().await;
            }
            let check = Arc::new(self.open(Arc::clone(&plugin), wanted));
            match check.validate().await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Err(rejected(&error)),
                Err(error) => return Err(ApiError::internal(error.to_string())),
            }
            checked = Some(check);
        }
        let plugin_id = plugin.id().to_string();
        let held = self
            .credentials
            .update(move |credentials| {
                credentials.plugins.insert(plugin_id, answers);
            })
            .await
            .map_err(|error| ApiError::internal(format!("database error: {error}")))?;
        // The copy that checked stays connected and runs from here.
        if let Some(check) = checked {
            self.lock().insert(plugin.id().to_string(), check);
        }
        self.running();
        Ok(info(&plugin, &held))
    }

    /// What the plugins turned on offer to run, in their manifests' order.
    pub fn actions(&self) -> Vec<api::PluginAction> {
        let mut running = self.running();
        running.sort_by(|a, b| a.plugin().id().cmp(b.plugin().id()));
        running
            .iter()
            .flat_map(|integration| {
                let plugin = integration.plugin();
                spec(plugin.manifest())
                    .actions
                    .iter()
                    .map(|action| api::PluginAction {
                        plugin: plugin.id().to_string(),
                        id: action.id.clone(),
                        label: action.label.clone(),
                        places: action.places.iter().map(|place| place_of(*place)).collect(),
                        ask: action.ask.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Run `plugin`'s action `id` on `target`, if it is turned on, lists the
    /// action, and offers it where `target` came from.
    pub async fn run_action(
        &self,
        plugin: &str,
        id: &str,
        target: api::ActionTarget,
        input: Option<String>,
    ) -> Result<Option<String>, ApiError> {
        let integration = self
            .running()
            .into_iter()
            .find(|integration| integration.plugin().id() == plugin)
            .ok_or_else(|| ApiError::not_found("that plugin is not turned on"))?;
        let manifest = integration.plugin();
        let action = spec(manifest.manifest())
            .actions
            .iter()
            .find(|action| action.id == id)
            .ok_or_else(|| ApiError::not_found("the plugin has no such action"))?;
        let wanted = match &target {
            api::ActionTarget::None => manifest::ActionPlace::Palette,
            api::ActionTarget::Tracks(_) => manifest::ActionPlace::Track,
            api::ActionTarget::Album(_) => manifest::ActionPlace::Album,
            api::ActionTarget::Artist(_) => manifest::ActionPlace::Artist,
        };
        if !action.places.contains(&wanted) {
            return Err(ApiError::invalid_input("the action is not offered there"));
        }
        let target = match target {
            api::ActionTarget::None => wit::ActionTarget::None,
            api::ActionTarget::Tracks(keys) => wit::ActionTarget::Tracks(keys),
            api::ActionTarget::Album(album) => wit::ActionTarget::Album(album),
            api::ActionTarget::Artist(artist) => wit::ActionTarget::Artist(artist),
        };
        match integration.run_action(id, &target, input.as_deref()).await {
            Ok(Ok(message)) => Ok(message),
            Ok(Err(message)) => Err(ApiError::invalid_input(message)),
            Err(error) => Err(ApiError::internal(error.to_string())),
        }
    }

    /// What the plugins turned on draw, in their manifests' order.
    pub fn panels(&self) -> Vec<api::PluginPanel> {
        let mut running = self.running();
        running.sort_by(|a, b| a.plugin().id().cmp(b.plugin().id()));
        running
            .iter()
            .flat_map(|integration| {
                let plugin = integration.plugin();
                spec(plugin.manifest())
                    .panels
                    .iter()
                    .map(|panel| api::PluginPanel {
                        plugin: plugin.id().to_string(),
                        id: panel.id.clone(),
                        title: panel.title.clone(),
                        places: panel
                            .places
                            .iter()
                            .map(|place| match place {
                                manifest::PanelPlace::Album => api::PanelPlace::Album,
                                manifest::PanelPlace::Artist => api::PanelPlace::Artist,
                            })
                            .collect(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Ask `plugin` to draw its panel `id` for `target`, if it is turned on
    /// and draws that panel on that kind of page.
    pub async fn render_panel(
        &self,
        plugin: &str,
        id: &str,
        target: api::PanelTarget,
    ) -> Result<Vec<api::PanelBlock>, ApiError> {
        let integration = self
            .running()
            .into_iter()
            .find(|integration| integration.plugin().id() == plugin)
            .ok_or_else(|| ApiError::not_found("that plugin is not turned on"))?;
        let owner = integration.plugin();
        let panel = spec(owner.manifest())
            .panels
            .iter()
            .find(|panel| panel.id == id)
            .ok_or_else(|| ApiError::not_found("the plugin has no such panel"))?;
        let (wanted, target) = match target {
            api::PanelTarget::Album(album) => {
                (manifest::PanelPlace::Album, wit::PanelTarget::Album(album))
            }
            api::PanelTarget::Artist(artist) => (
                manifest::PanelPlace::Artist,
                wit::PanelTarget::Artist(artist),
            ),
        };
        if !panel.places.contains(&wanted) {
            return Err(ApiError::invalid_input("the panel is not drawn there"));
        }
        match integration.render_panel(id, &target).await {
            Ok(Ok(blocks)) => Ok(blocks.into_iter().take(MAX_BLOCKS).map(block_of).collect()),
            Ok(Err(message)) => Err(ApiError::invalid_input(message)),
            Err(error) => Err(ApiError::internal(error.to_string())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Integration>>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Forget every answer, which turns the plugin off.
    pub async fn clear(&self, id: &str) -> Result<(), ApiError> {
        let plugin = plugin_of(id).ok_or_else(|| ApiError::not_found("no such integration"))?;
        self.forget(plugin.id()).await
    }

    /// Forget every answer for the plugin `id`, loaded or not, so an
    /// uninstalled plugin leaves no secret behind and a reinstall starts
    /// turned off.
    pub async fn forget(&self, id: &str) -> Result<(), ApiError> {
        let _edit = self.edits.lock().await;
        let plugin_id = id.to_string();
        self.credentials
            .update(move |credentials| {
                credentials.plugins.remove(&plugin_id);
            })
            .await
            .map_err(|error| ApiError::internal(format!("database error: {error}")))?;
        self.running();
        Ok(())
    }

    /// The running integrations, brought in step with the registry and the
    /// answers first: a new build of a plugin is swapped in, changed answers
    /// reopen it, and one turned off or no longer loaded is closed.
    pub fn running(&self) -> Vec<Arc<Integration>> {
        let credentials = self.credentials.snapshot();
        let mut running = self.lock();
        let mut kept = HashMap::new();
        let mut retired = Vec::new();
        for plugin in loaded() {
            let integration = spec(plugin.manifest());
            let answers = credentials.plugin(plugin.id());
            if !runs(integration, &answers) {
                continue;
            }
            let wanted = settings(integration, &answers);
            let live = match running.remove(plugin.id()) {
                Some(live) if live.settings() == wanted.as_slice() && !live.is_closed() => {
                    if !Arc::ptr_eq(&live.plugin(), &plugin) {
                        live.swap(Arc::clone(&plugin));
                    }
                    live
                }
                old => {
                    retired.extend(old);
                    Arc::new(self.open(Arc::clone(&plugin), wanted))
                }
            };
            kept.insert(plugin.id().to_string(), live);
        }
        retired.extend(running.drain().map(|(_, live)| live));
        *running = kept;
        // By id, so a plugin asked before another for lyrics or radio is always asked first.
        let mut current: Vec<(&String, &Arc<Integration>)> = running.iter().collect();
        current.sort_by_key(|(id, _)| *id);
        let current = current
            .into_iter()
            .map(|(_, live)| Arc::clone(live))
            .collect();
        drop(running);
        retire(retired);
        current
    }
}

/// Close what no longer runs, so a queue still holding one cannot keep its
/// connection open. Without a runtime nothing was ever opened to close.
fn retire(retired: Vec<Arc<Integration>>) {
    if retired.is_empty() {
        return;
    }
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(async move {
            for integration in retired {
                integration.close().await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lights() -> manifest::Integration {
        Manifest::parse(
            r#"
            id = "lights"
            name = "Lights"
            version = "0.1.0"
            api = "0.2"

            [integration]
            server = "url"

            [[integration.field]]
            key = "url"
            label = "Address"
            kind = "url"
            default = "ws://127.0.0.1:12345"
            required = true

            [[integration.field]]
            key = "token"
            label = "Token"
            kind = "secret"
            required = true

            [[integration.field]]
            key = "strobe"
            label = "Strobe"
            kind = "toggle"
            "#,
        )
        .expect("manifest")
        .integration
        .expect("an integration")
    }

    fn answer(key: &str, value: &str) -> FieldValue {
        FieldValue::new(key, value)
    }

    #[test]
    fn a_default_answers_until_the_person_types_and_a_secret_never_has_one() {
        let integration = lights();
        let opened = settings(&integration, &PluginAnswers::new());
        assert_eq!(
            opened,
            vec![
                ("url".to_string(), "ws://127.0.0.1:12345".to_string()),
                ("token".to_string(), String::new()),
                ("strobe".to_string(), "false".to_string()),
            ]
        );
        assert!(!complete(&integration, &opened), "the token is required");
    }

    #[test]
    fn it_runs_only_once_turned_on_with_every_required_field_filled() {
        let integration = lights();
        let token = merge(
            &integration,
            PluginAnswers::new(),
            &[answer("token", "hunter2")],
        )
        .expect("merged");
        assert!(!runs(&integration, &token), "not turned on yet");
        let on = merge(&integration, token, &[answer(ENABLED, "true")]).expect("merged");
        assert!(runs(&integration, &on));
        let emptied = merge(&integration, on.clone(), &[answer("url", "  ")]).expect("merged");
        assert!(
            runs(&integration, &emptied),
            "an emptied address falls back to its default"
        );
        let off = merge(&integration, on, &[answer(ENABLED, "false")]).expect("merged");
        assert!(!runs(&integration, &off));
    }

    #[test]
    fn a_secret_reads_as_kept_only_once_one_is_stored() {
        let integration = lights();
        assert!(!secrets_held(&integration, &PluginAnswers::new()));
        let stored = PluginAnswers::from([("token".to_string(), "hunter2".to_string())]);
        assert!(secrets_held(&integration, &stored));
        let none = Manifest::parse(
            "id = \"x\"\nname = \"X\"\nversion = \"0.1.0\"\napi = \"0.2\"\n[integration]\n",
        )
        .expect("manifest")
        .integration
        .expect("integration");
        assert!(
            secrets_held(&none, &PluginAnswers::new()),
            "nothing to hold"
        );
        let optional = Manifest::parse(
            "id = \"x\"\nname = \"X\"\nversion = \"0.1.0\"\napi = \"0.2\"\n[integration]\n\
             [[integration.field]]\nkey = \"token\"\nlabel = \"Token\"\nkind = \"secret\"\n",
        )
        .expect("manifest")
        .integration
        .expect("integration");
        assert!(
            secrets_held(&optional, &PluginAnswers::new()),
            "an optional secret left empty still reads as set up"
        );
    }

    #[test]
    fn an_empty_secret_keeps_the_stored_one_and_an_unknown_key_is_refused() {
        let integration = lights();
        let stored = PluginAnswers::from([("token".to_string(), "hunter2".to_string())]);
        let kept = merge(&integration, stored, &[answer("token", "")]).expect("merged");
        assert_eq!(kept.get("token").map(String::as_str), Some("hunter2"));
        assert!(merge(&integration, kept, &[answer("color", "red")]).is_err());
    }
}

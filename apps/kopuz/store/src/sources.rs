use std::collections::HashMap;

use api::{
    ApiEvent, NetworkStatus, OfflineMode, SourceCapabilities, SourceInfo, SourceState, Table,
};
use gpui_kit::{App, Context, Entity, EventEmitter, Subscription, Task, Window};
use link::Link;

use crate::config::ConfigModel;
use crate::invalidation::{Invalidation, TablesChanged};
use crate::notices::Notices;

/// What a source's row says about the link to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceStatus {
    Connected,
    Connecting,
    Disconnected,
}

impl SourceStatus {
    /// The daemon's probe answer as a status; no answer yet reads as connecting.
    pub fn of(state: Option<SourceState>) -> Self {
        match state {
            Some(SourceState::Online) => Self::Connected,
            Some(SourceState::AuthExpired | SourceState::Offline) => Self::Disconnected,
            Some(SourceState::Checking) | None => Self::Connecting,
        }
    }
}

/// What the app says about the internet, if anything, given the daemon's network status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkNotice {
    /// Only sources on this machine or its network work until the connection is back.
    Down,
    /// The connection is back, and `source` is the one to offer going back to.
    BackOnline { source: String },
}

impl NetworkNotice {
    /// Down while the checks fail; once they pass, the source the daemon left, or the one offline mode held.
    pub fn of(status: &NetworkStatus, active: Option<&str>) -> Option<Self> {
        if status.internet_down {
            return Some(Self::Down);
        }
        if let Some(left) = &status.left_source {
            return Some(Self::BackOnline {
                source: left.clone(),
            });
        }
        match status.offline_mode {
            OfflineMode::Automatic => active.map(|source| Self::BackOnline {
                source: source.to_string(),
            }),
            OfflineMode::Off | OfflineMode::Requested => None,
        }
    }
}

/// Configured sources and the active one's capabilities, which gate every affordance.
pub struct SourcesModel {
    sources: Vec<SourceInfo>,
    /// The daemon's latest probe answers, laid over each fetch: a fetch that left before an answer carries the older one.
    heard: HashMap<String, SourceState>,
    loaded: bool,
    /// Whether the daemon reaches the internet, which decides what a remote source can do.
    network: NetworkStatus,
    /// `--offline` asked for offline mode before the daemon answered.
    offline_on_connect: bool,
    _network: Option<Task<()>>,
    _fetch: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// A switch of source or of offline mode was asked for: what is on screen is about to stop being true.
pub struct SwitchStarted;

impl EventEmitter<SwitchStarted> for SourcesModel {}

impl SourcesModel {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let invalidation = Invalidation::global(cx);
        let subscriptions = vec![
            cx.subscribe(&invalidation, |this, _, event: &TablesChanged, cx| {
                if event.touches(&[Table::Servers]) {
                    this.refetch(cx);
                }
            }),
            cx.subscribe(&Link::connection(cx), |this, _, event: &ApiEvent, cx| {
                match event {
                    ApiEvent::SourceStatus { source, state } => {
                        this.heard.insert(source.clone(), *state);
                        if let Some(row) = this.sources.iter_mut().find(|row| &row.id == source) {
                            row.state = Some(*state);
                            crate::notify(cx);
                        }
                    }
                    // Answers may have been lost; the refetch the resync starts is trusted over them.
                    ApiEvent::Resync => {
                        this.heard.clear();
                        match std::mem::take(&mut this.offline_on_connect) {
                            true => this.set_offline(true, cx),
                            false => this.read_network(cx),
                        }
                    }
                    ApiEvent::NetworkStatus(status) => {
                        this.network = status.clone();
                        crate::notify(cx);
                    }
                    _ => {}
                }
            }),
        ];
        Self {
            sources: Vec::new(),
            heard: HashMap::new(),
            loaded: false,
            network: NetworkStatus::default(),
            offline_on_connect: false,
            _network: None,
            _fetch: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn refetch(&mut self, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.sources().await });
        self._fetch = Some(cx.spawn(async move |this, cx| {
            let Ok(sources) = call.await else { return };
            this.update(cx, |this, cx| this.adopt(sources, cx)).ok();
        }));
    }

    fn adopt(&mut self, mut sources: Vec<SourceInfo>, cx: &mut Context<Self>) {
        let before = self.active().map(|source| source.id.clone());
        for row in &mut sources {
            if let Some(state) = self.heard.get(&row.id) {
                row.state = Some(*state);
            }
        }
        self.sources = sources;
        let after = self.active().map(|source| source.id.clone());
        let switched = self.loaded && before != after;
        self.loaded = true;
        crate::notify(cx);
        if switched {
            Invalidation::global(cx).update(cx, |invalidation, cx| invalidation.invalidate_all(cx));
        }
    }

    pub fn network(&self) -> &NetworkStatus {
        &self.network
    }

    /// What the app should say about the internet now.
    pub fn network_notice(&self) -> Option<NetworkNotice> {
        let active = self.active().map(|source| source.id.as_str());
        NetworkNotice::of(&self.network, active)
    }

    /// The internet is down and `source` needs it; the daemon would refuse switching to it.
    pub fn cut_off(&self, source: &SourceInfo) -> bool {
        cut_off(&self.network, source)
    }

    /// Offline mode on a source across the internet, whose listings the daemon keeps to the songs with a copy here.
    pub fn offline_reads(&self) -> bool {
        self.network.offline_mode.is_on()
            && self.active().is_some_and(|source| !source.works_offline)
    }

    fn read_network(&mut self, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.network_status().await });
        self._network = Some(cx.spawn(async move |this, cx| {
            let Ok(status) = call.await else { return };
            this.update(cx, |this, cx| {
                this.network = status;
                crate::notify(cx);
            })
            .ok();
        }));
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn sources(&self) -> &[SourceInfo] {
        &self.sources
    }

    pub fn active(&self) -> Option<&SourceInfo> {
        self.sources.iter().find(|source| source.active)
    }

    /// The active source's link as the daemon's probe last found it.
    pub fn active_status(&self) -> SourceStatus {
        SourceStatus::of(self.active().and_then(|source| source.state))
    }

    /// Whether the active source has a catalog of its own that it can reach now, which Home then leads with.
    pub fn has_catalog(&self) -> bool {
        let caps = self.active().map(|source| &source.capabilities);
        let offers = caps.is_some_and(|caps| caps.discover || !caps.pages.is_empty());
        offers && !self.offline_reads()
    }

    /// The active source's capabilities; all off until sources load.
    pub fn capabilities(&self) -> SourceCapabilities {
        self.active()
            .map(|source| source.capabilities.clone())
            .unwrap_or_default()
    }

    /// Make `id` active, and start whichever sign-in the daemon says it still needs.
    pub fn switch(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.active().is_none_or(|source| source.id != id) {
            cx.emit(SwitchStarted);
        }
        let call = Link::global(cx).call(move |api| async move { api.switch_source(id).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = call.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(source) => {
                    this.refetch(cx);
                    ConfigModel::global(cx).update(cx, |config, cx| config.reload(cx));
                    crate::signin::start_sign_in(&source, window, cx);
                }
                Err(error) => Notices::failed(&error, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Enter offline mode, or leave it however it was entered.
    pub fn set_offline(&mut self, on: bool, cx: &mut Context<Self>) {
        // Only a source that needs the internet reads differently in offline mode.
        let remote = self.active().is_some_and(|source| !source.works_offline);
        if remote && on != self.network.offline_mode.is_on() {
            cx.emit(SwitchStarted);
        }
        let call = Link::global(cx).call(move |api| async move { api.set_offline(on).await });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| match result {
                Ok(status) => {
                    let said = match status.offline_mode.is_on() {
                        true => "offline_mode_on",
                        false => "offline_mode_off",
                    };
                    this.network = status;
                    crate::notify(cx);
                    Notices::info(i18n::t(said), cx);
                }
                Err(error) => Notices::failed(&error, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Ask for offline mode as soon as the daemon answers, as `--offline` does.
    pub fn offline_on_connect(&mut self, cx: &mut Context<Self>) {
        match Link::connection(cx).read(cx).is_connected() {
            true => self.set_offline(true, cx),
            false => self.offline_on_connect = true,
        }
    }

    /// Leave offline mode and go back to `source`, unless it is the active one already.
    pub fn back_online(&mut self, source: String, window: &mut Window, cx: &mut Context<Self>) {
        let leave = self.network.offline_mode.is_on();
        let switch = self.active().is_none_or(|active| active.id != source);
        let call = Link::global(cx).call(move |api| async move {
            if leave {
                api.set_offline(false).await?;
            }
            Ok(())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = call.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) if switch => this.switch(source, window, cx),
                Ok(()) => this.read_network(cx),
                Err(error) => Notices::failed(&error, cx),
            })
            .ok();
        })
        .detach();
    }
}

/// Whether the internet being down leaves `source` out of reach; a folder or a loopback or LAN server is not.
pub fn cut_off(status: &NetworkStatus, source: &SourceInfo) -> bool {
    status.internet_down && !source.works_offline && !source.active
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(down: bool, left: Option<&str>, mode: OfflineMode) -> NetworkStatus {
        NetworkStatus {
            internet_down: down,
            left_source: left.map(str::to_string),
            offline_mode: mode,
        }
    }

    fn back(source: &str) -> Option<NetworkNotice> {
        Some(NetworkNotice::BackOnline {
            source: source.into(),
        })
    }

    #[test]
    fn a_down_internet_is_said_whatever_else_holds() {
        for mode in [
            OfflineMode::Off,
            OfflineMode::Automatic,
            OfflineMode::Requested,
        ] {
            for left in [None, Some("yt")] {
                assert_eq!(
                    NetworkNotice::of(&status(true, left, mode), Some("local")),
                    Some(NetworkNotice::Down)
                );
            }
        }
    }

    #[test]
    fn once_back_it_offers_the_source_left_or_the_one_offline_mode_held() {
        let of = |left, mode| NetworkNotice::of(&status(false, left, mode), Some("yt"));
        assert_eq!(of(Some("sc"), OfflineMode::Off), back("sc"));
        assert_eq!(of(Some("sc"), OfflineMode::Automatic), back("sc"));
        assert_eq!(of(None, OfflineMode::Automatic), back("yt"));
    }

    #[test]
    fn nothing_is_said_when_online_or_offline_by_request() {
        let of = |mode, active| NetworkNotice::of(&status(false, None, mode), active);
        assert_eq!(of(OfflineMode::Off, Some("yt")), None);
        assert_eq!(of(OfflineMode::Requested, Some("yt")), None);
        assert_eq!(of(OfflineMode::Automatic, None), None);
    }

    #[test]
    fn only_a_source_across_the_internet_that_is_not_active_is_cut_off() {
        let source = |works_offline, active| SourceInfo {
            needs_network: true,
            works_offline,
            active,
            ..SourceInfo::default()
        };
        let down = status(true, None, OfflineMode::Off);
        let up = status(false, None, OfflineMode::Off);
        assert!(cut_off(&down, &source(false, false)));
        assert!(!cut_off(&down, &source(false, true)));
        assert!(!cut_off(&down, &source(true, false)));
        assert!(!cut_off(&up, &source(false, false)));
    }
}

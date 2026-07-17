//! Browser-hosted Spotify playback.
//!
//! The Web Playback SDK needs Widevine DRM and only behaves reliably in
//! Chromium-family browsers, so the host runs a tiny localhost server, opens a
//! Chromium browser at a page that boots `Spotify.Player`, and drives it over
//! a WebSocket (the page still warns if the browser it lands in lacks DRM):
//!
//! - `GET /`   → serves [`PLAYER_PAGE`] (raw HTTP).
//! - `GET /ws` → upgrades to a WebSocket.
//!
//! kopuz → browser: JSON command frames (`pause`/`resume`/`seek`/`set_volume`/
//! `set_token`/`disconnect`). browser → kopuz: [`HostEvent`]s (`ready`, `state`,
//! `activated`, `error`, …), broadcast to subscribers.

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, broadcast};
use tokio_tungstenite::tungstenite::Message;

/// Events reported by the browser player, fanned out to subscribers.
#[derive(Debug, Clone)]
pub enum HostEvent {
    /// The SDK registered a Connect device we can target for playback.
    Ready { device_id: String },
    /// The device went offline.
    NotReady,
    /// A player-state tick.
    State {
        paused: bool,
        position_ms: u64,
        duration_ms: u64,
        track_id: Option<String>,
        /// Heuristic end-of-track (SDK reports paused at position 0 after play).
        ended: bool,
    },
    /// The user clicked the page's enable-playback button (autoplay gesture).
    Activated,
    /// A media-key action captured by the tab's Media Session (OS now-playing
    /// widget / keyboard media keys target the browser, since it owns the
    /// audio). `action` is `play`/`pause`/`next`/`prev`/`seek`; kopuz routes it
    /// through its own queue — the SDK only ever holds one track.
    Media {
        action: String,
        position_ms: Option<u64>,
    },
    /// A player error. `kind` is one of `account`/`auth`/`widevine`/`playback`/
    /// `license` (DRM license failures — tracks dying ~10s in; the message is
    /// already user-facing).
    Error { kind: String, message: String },
}

/// Handle to a running browser playback host. Cheap to clone (shared channels).
#[derive(Clone)]
pub struct SpotifyHost {
    /// JSON command frames pushed to the connected browser tab.
    cmd_tx: broadcast::Sender<String>,
    /// Player events fanned out to kopuz subscribers.
    event_tx: broadcast::Sender<HostEvent>,
    /// The current OAuth token, re-sent to each new tab and on rotation.
    token: Arc<Mutex<String>>,
}

impl SpotifyHost {
    /// Bind an ephemeral localhost port, start serving, and open the page in
    /// the user's chosen browser — or the first available supported one (see
    /// [`open_player_page`] for why never the system default).
    pub async fn start(access: String, browser: Option<String>) -> Result<Self, String> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("couldn't bind localhost for Spotify playback: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();

        let (cmd_tx, _) = broadcast::channel::<String>(64);
        let (event_tx, _) = broadcast::channel::<HostEvent>(256);
        let token = Arc::new(Mutex::new(access));

        let host = Self {
            cmd_tx: cmd_tx.clone(),
            event_tx: event_tx.clone(),
            token: token.clone(),
        };

        tokio::spawn(accept_loop(listener, cmd_tx, event_tx, token));

        let url = format!("http://127.0.0.1:{port}/");
        open_player_page(&url, browser.as_deref())?;

        Ok(host)
    }

    /// Subscribe to player events.
    pub fn subscribe(&self) -> broadcast::Receiver<HostEvent> {
        self.event_tx.subscribe()
    }

    fn send(&self, cmd: Value) {
        let _ = self.cmd_tx.send(cmd.to_string());
    }

    pub fn pause(&self) {
        self.send(json!({ "cmd": "pause" }));
    }

    pub fn resume(&self) {
        self.send(json!({ "cmd": "resume" }));
    }

    pub fn seek(&self, position_ms: u64) {
        self.send(json!({ "cmd": "seek", "position_ms": position_ms }));
    }

    pub fn set_volume(&self, volume: f32) {
        self.send(json!({ "cmd": "set_volume", "volume": volume }));
    }

    pub fn disconnect(&self) {
        self.send(json!({ "cmd": "disconnect" }));
    }

    /// Update the OAuth token (kept for new tabs, and pushed to the live one so
    /// the SDK's `getOAuthToken` callback always has a fresh token).
    pub async fn set_token(&self, access: String) {
        *self.token.lock().await = access.clone();
        self.send(json!({ "cmd": "set_token", "token": access }));
    }
}

/// A browser that can host the player page, offered in the settings picker.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlayerBrowser {
    /// Stable id persisted in `AppConfig::spotify_browser`.
    pub id: &'static str,
    /// Display name for the picker.
    pub label: &'static str,
}

#[cfg(target_os = "macos")]
const BROWSERS: &[(PlayerBrowser, &str, &str)] = &[
    (
        PlayerBrowser {
            id: "chrome",
            label: "Google Chrome",
        },
        "com.google.Chrome",
        "Google Chrome.app",
    ),
    (
        PlayerBrowser {
            id: "edge",
            label: "Microsoft Edge",
        },
        "com.microsoft.edgemac",
        "Microsoft Edge.app",
    ),
    (
        PlayerBrowser {
            id: "brave",
            label: "Brave",
        },
        "com.brave.Browser",
        "Brave Browser.app",
    ),
    (
        PlayerBrowser {
            id: "chromium",
            label: "Chromium",
        },
        "org.chromium.Chromium",
        "Chromium.app",
    ),
    (
        PlayerBrowser {
            id: "vivaldi",
            label: "Vivaldi",
        },
        "com.vivaldi.Vivaldi",
        "Vivaldi.app",
    ),
    (
        PlayerBrowser {
            id: "safari",
            label: "Safari",
        },
        "com.apple.Safari",
        "Safari.app",
    ),
];

#[cfg(all(unix, not(target_os = "macos")))]
const BROWSERS: &[(PlayerBrowser, &[&str])] = &[
    (
        PlayerBrowser {
            id: "chrome",
            label: "Google Chrome",
        },
        &["google-chrome-stable", "google-chrome"],
    ),
    (
        PlayerBrowser {
            id: "chromium",
            label: "Chromium",
        },
        &["chromium", "chromium-browser"],
    ),
    (
        PlayerBrowser {
            id: "brave",
            label: "Brave",
        },
        &["brave-browser", "brave"],
    ),
    (
        PlayerBrowser {
            id: "edge",
            label: "Microsoft Edge",
        },
        &["microsoft-edge", "microsoft-edge-stable"],
    ),
    (
        PlayerBrowser {
            id: "vivaldi",
            label: "Vivaldi",
        },
        &["vivaldi"],
    ),
];

#[cfg(target_os = "macos")]
fn browser_installed(app_name: &str) -> bool {
    let candidates = [
        format!("/Applications/{app_name}"),
        format!("/System/Applications/{app_name}"),
    ];
    if candidates.iter().any(|p| std::path::Path::new(p).exists()) {
        return true;
    }
    std::env::var_os("HOME").is_some_and(|home| {
        std::path::Path::new(&home)
            .join("Applications")
            .join(app_name)
            .exists()
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn command_in_path(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(cmd).exists()))
}

/// The playback-capable browsers installed on this machine, in the order the
/// automatic choice tries them.
pub fn available_browsers() -> Vec<PlayerBrowser> {
    #[cfg(target_os = "macos")]
    {
        BROWSERS
            .iter()
            .filter(|(_, _, app)| browser_installed(app))
            .map(|(b, _, _)| *b)
            .collect()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        BROWSERS
            .iter()
            .filter(|(_, cmds)| cmds.iter().any(|c| command_in_path(c)))
            .map(|(b, _)| *b)
            .collect()
    }
    #[cfg(target_os = "windows")]
    {
        let mut out = Vec::new();
        if webbrowser::Browser::Chrome.exists() {
            out.push(PlayerBrowser {
                id: "chrome",
                label: "Google Chrome",
            });
        }
        out.push(PlayerBrowser {
            id: "edge",
            label: "Microsoft Edge",
        });
        out
    }
}

fn launch_browser(id: &str, url: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        let Some((_, bundle, _)) = BROWSERS.iter().find(|(b, _, _)| b.id == id) else {
            return false;
        };
        std::process::Command::new("open")
            .args(["-b", bundle, url])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let Some((_, cmds)) = BROWSERS.iter().find(|(b, _)| b.id == id) else {
            return false;
        };
        cmds.iter()
            .any(|cmd| std::process::Command::new(cmd).arg(url).spawn().is_ok())
    }
    #[cfg(target_os = "windows")]
    {
        match id {
            "chrome" => webbrowser::open_browser(webbrowser::Browser::Chrome, url).is_ok(),
            "edge" => std::process::Command::new("cmd")
                .args(["/C", "start", "", &format!("microsoft-edge:{url}")])
                .status()
                .map(|s| s.success())
                .unwrap_or(false),
            _ => false,
        }
    }
}

/// Open the player page in the user's chosen browser, or the first available
/// one — deliberately never the system default: the SDK is only reliable in
/// Chromium-family browsers and Safari (Firefox has a long-standing playback
/// bug, spotify/web-playback-sdk#116, and ignores media-session metadata).
fn open_player_page(url: &str, preferred: Option<&str>) -> Result<(), String> {
    if let Some(id) = preferred {
        if launch_browser(id, url) {
            tracing::info!(browser = id, "spotify player page opened");
            return Ok(());
        }
        tracing::warn!(
            browser = id,
            "chosen spotify browser unavailable; trying others"
        );
    }
    for browser in available_browsers() {
        if preferred == Some(browser.id) {
            continue;
        }
        if launch_browser(browser.id, url) {
            tracing::info!(browser = browser.id, "spotify player page opened");
            return Ok(());
        }
    }
    Err(
        "Spotify playback needs Chrome, Edge, Brave, Chromium, Vivaldi, or Safari — \
         none was found. Install one (or pick a browser in Settings) and play the \
         track again."
            .to_string(),
    )
}

async fn accept_loop(
    listener: tokio::net::TcpListener,
    cmd_tx: broadcast::Sender<String>,
    event_tx: broadcast::Sender<HostEvent>,
    token: Arc<Mutex<String>>,
) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let cmd_tx = cmd_tx.clone();
        let event_tx = event_tx.clone();
        let token = token.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_stream(stream, cmd_tx, event_tx, token).await {
                tracing::debug!(error = %e, "spotify host connection ended");
            }
        });
    }
}

async fn handle_stream(
    mut stream: TcpStream,
    cmd_tx: broadcast::Sender<String>,
    event_tx: broadcast::Sender<HostEvent>,
    token: Arc<Mutex<String>>,
) -> Result<(), String> {
    let mut peek = [0u8; 2048];
    let n = stream.peek(&mut peek).await.map_err(|e| e.to_string())?;
    let head = String::from_utf8_lossy(&peek[..n]);
    let path = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    let is_ws = head.to_ascii_lowercase().contains("upgrade: websocket");

    if is_ws {
        let ws = tokio_tungstenite::accept_async(stream)
            .await
            .map_err(|e| e.to_string())?;
        run_ws(ws, cmd_tx, event_tx, token).await;
        Ok(())
    } else {
        let mut scratch = vec![0u8; n.max(1)];
        let _ = stream.read(&mut scratch).await;
        serve_page(&mut stream, &path).await;
        Ok(())
    }
}

async fn serve_page(stream: &mut TcpStream, path: &str) {
    let (status, body, ctype) = if path == "/" || path.starts_with("/?") {
        ("200 OK", PLAYER_PAGE, "text/html; charset=utf-8")
    } else {
        ("404 Not Found", "", "text/plain")
    };
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn run_ws(
    ws: tokio_tungstenite::WebSocketStream<TcpStream>,
    cmd_tx: broadcast::Sender<String>,
    event_tx: broadcast::Sender<HostEvent>,
    token: Arc<Mutex<String>>,
) {
    let (mut sink, mut source) = ws.split();

    let initial = {
        let t = token.lock().await.clone();
        json!({ "cmd": "set_token", "token": t }).to_string()
    };
    if sink.send(Message::text(initial)).await.is_err() {
        return;
    }

    let mut cmd_rx = cmd_tx.subscribe();
    let forward = tokio::spawn(async move {
        while let Ok(msg) = cmd_rx.recv().await {
            if sink.send(Message::text(msg)).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = source.next().await {
        if let Message::Text(text) = msg
            && let Ok(val) = serde_json::from_str::<Value>(text.as_str())
        {
            emit_event(&val, &event_tx);
        }
    }

    forward.abort();
}

fn emit_event(val: &Value, event_tx: &broadcast::Sender<HostEvent>) {
    let Some(event) = val["event"].as_str() else {
        return;
    };
    if event == "log" {
        tracing::info!(target: "spotify_player_page", "{}", val["line"].as_str().unwrap_or_default());
        return;
    }
    let out = match event {
        "ready" => HostEvent::Ready {
            device_id: val["device_id"].as_str().unwrap_or_default().to_string(),
        },
        "not_ready" => HostEvent::NotReady,
        "activated" => HostEvent::Activated,
        "media" => HostEvent::Media {
            action: val["action"].as_str().unwrap_or_default().to_string(),
            position_ms: val["position_ms"].as_u64(),
        },
        "state" => HostEvent::State {
            paused: val["paused"].as_bool().unwrap_or(true),
            position_ms: val["position"].as_u64().unwrap_or(0),
            duration_ms: val["duration"].as_u64().unwrap_or(0),
            track_id: val["track_id"].as_str().map(str::to_string),
            ended: val["ended"].as_bool().unwrap_or(false),
        },
        "error" => HostEvent::Error {
            kind: val["kind"].as_str().unwrap_or("unknown").to_string(),
            message: val["message"].as_str().unwrap_or_default().to_string(),
        },
        _ => return,
    };
    let _ = event_tx.send(out);
}

/// The page served to the browser: boots the Web Playback SDK, connects back to
/// kopuz over a WebSocket, and bridges commands ⇄ events.
const PLAYER_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>kopuz · Spotify</title>
<style>
  body { font-family: system-ui, sans-serif; background: #121212; color: #fff;
         text-align: center; padding-top: 3rem; margin: 0; }
  h2 { font-weight: 600; }
  button { background: #1db954; border: 0; color: #fff; padding: 0.9rem 2rem;
           border-radius: 2rem; font-size: 1rem; cursor: pointer; margin-top: 1rem; }
  button:hover { background: #1ed760; }
  #status { margin-top: 1.25rem; color: #b3b3b3; }
  #log { margin: 1.5rem auto 0; max-width: 46rem; text-align: left; font-size: 12px;
         line-height: 1.5; color: #9aa; background: #0a0a0a; border: 1px solid #222;
         border-radius: 8px; padding: 0.75rem 1rem; max-height: 40vh; overflow: auto;
         white-space: pre-wrap; }
</style>
</head>
<body>
  <h2>kopuz · Spotify playback</h2>
  <button id="activate">Click to enable playback</button>
  <p id="status">Loading the Spotify player…</p>
  <pre id="log"></pre>
  <script>
    let ws = null, token = "", player = null;
    let wasPlaying = false, reportedEnded = false, curId = null, nearEnd = false;
    let lastPos = -1, lastPosAt = 0, tokenCalls = 0;
    let prevPaused = true, deathRetries = 0, lastPauseCmdAt = 0, hasPlayed = false;
    let errTimes = [], stallResumes = 0, autoplayBlocked = false, isReady = false;
    let unexpectedPauseAt = 0, lastDeathAt = 0, userActivated = false;
    const IS_FIREFOX = /firefox/i.test(navigator.userAgent);
    const LICENSE_HINT = IS_FIREFOX
      ? "Spotify playback keeps failing — Firefox has a known Spotify player bug. " +
        "Copy this page's address into Chrome, Edge, or Brave."
      : "Spotify playback keeps failing in this browser (DRM/license errors). " +
        "Try Chrome, Edge, or Brave, or another network (VPN off).";
    const statusEl = document.getElementById("status");
    const logEl = document.getElementById("log");
    const setStatus = (t) => { statusEl.textContent = t; };
    const logLine = (t) => {
      const ts = new Date().toLocaleTimeString();
      logEl.textContent = (ts + "  " + t + "\n" + logEl.textContent).slice(0, 6000);
      send({ event: "log", line: t });
    };
    const send = (o) => { try { if (ws && ws.readyState === 1) ws.send(JSON.stringify(o)); } catch (e) {} };

    (() => {
      const interesting = (u) => /license|widevine|scdn|spclient|spotify/i.test(u);
      const origFetch = window.fetch;
      window.fetch = function (input) {
        const url = (typeof input === "string") ? input : (input && input.url) || "";
        const p = origFetch.apply(this, arguments);
        if (interesting(url)) {
          p.then(
            (r) => { if (!r.ok) logLine("NET " + r.status + " " + url.slice(0, 120)); },
            (e) => logLine("NET FAIL " + url.slice(0, 120) + " — " + e)
          );
        }
        return p;
      };
      const origOpen = XMLHttpRequest.prototype.open;
      XMLHttpRequest.prototype.open = function (method, url) {
        if (interesting(String(url))) {
          this.addEventListener("loadend", () => {
            if (this.status === 0 || this.status >= 400)
              logLine("NET(xhr) " + this.status + " " + String(url).slice(0, 120));
          });
        }
        return origOpen.apply(this, arguments);
      };
    })();

    function probeAutoplay() {
      const done = (ok, why) => {
        logLine("autoplay " + (ok ? "allowed" : "blocked" + (why ? " (" + why + ")" : "")));
        if (userActivated) return;
        autoplayBlocked = !ok;
        if (ok) { send({ event: "activated" }); setStatus("Ready — audio plays in this tab."); }
        else { setStatus("Click the button to enable playback."); }
      };
      try {
        const a = new Audio("data:audio/wav;base64,UklGRiQAAABXQVZFZm10IBAAAAABAAEAQB8AAIA+AAACABAAZGF0YQAAAAA=");
        const p = a.play();
        if (!p || !p.then) { done(true); return; }
        p.then(() => { a.pause(); done(true); }).catch((e) => done(false, e && e.name));
      } catch (e) { done(false, e && e.name); }
    }

    function claimMediaSession(cur) {
      if (!("mediaSession" in navigator)) return;
      const ms = navigator.mediaSession;
      const forward = (action, extra) => () => send(Object.assign({ event: "media", action }, extra));
      try {
        ms.setActionHandler("play", forward("play"));
        ms.setActionHandler("pause", forward("pause"));
        ms.setActionHandler("nexttrack", forward("next"));
        ms.setActionHandler("previoustrack", forward("prev"));
        ms.setActionHandler("seekto", (d) => {
          if (d && d.seekTime != null) send({ event: "media", action: "seek", position_ms: Math.round(d.seekTime * 1000) });
        });
      } catch (e) {}
      ensureMediaMetadata(cur);
    }

    let lastCur = null, lastMetaTitle = null;
    function ensureMediaMetadata(cur) {
      if (cur) lastCur = cur;
      cur = cur || lastCur;
      if (!cur || !("mediaSession" in navigator) || typeof MediaMetadata === "undefined") return;
      const ms = navigator.mediaSession;
      const title = cur.name || "";
      if (title !== lastMetaTitle) {
        lastMetaTitle = title;
        logLine("media session metadata -> " + title);
      }
      try {
        ms.metadata = new MediaMetadata({
          title,
          artist: (cur.artists || []).map((a) => a.name).join(", "),
          album: (cur.album && cur.album.name) || "",
          artwork: ((cur.album && cur.album.images) || []).map((i) => ({
            src: i.url,
            sizes: (i.width || 300) + "x" + (i.height || 300),
            type: "image/jpeg",
          })),
        });
      } catch (e) {}
    }

    let wsRetries = 0;
    function connect() {
      ws = new WebSocket("ws://" + location.host + "/ws");
      ws.onopen = () => { wsRetries = 0; };
      ws.onmessage = (m) => { let d; try { d = JSON.parse(m.data); } catch (e) { return; } onCommand(d); };
      ws.onclose = () => {
        wsRetries += 1;
        if (wsRetries < 5) { setTimeout(connect, 1000); return; }
        if (player) {
          try { player.pause(); } catch (e) {}
          try { player.disconnect(); } catch (e) {}
        }
        setStatus("kopuz closed — this tab can be closed.");
        window.close();
      };
    }

    function onCommand(d) {
      if (d.cmd !== "set_token") logLine("cmd " + d.cmd + (d.position_ms != null ? " " + d.position_ms : ""));
      if (d.cmd === "set_token") {
        const fresh = (d.token || "") && (d.token || "") !== token;
        token = d.token || "";
        if (!player && window.Spotify) boot();
        else if (player && fresh && !isReady) {
          logLine("token rotated while disconnected — reconnecting SDK");
          player.connect().then((ok) => logLine("connect() -> " + ok));
        }
      } else if (!player) {
        return;
      } else if (d.cmd === "pause") { lastPauseCmdAt = Date.now(); player.pause(); }
      else if (d.cmd === "resume") { player.resume(); }
      else if (d.cmd === "seek") { player.seek(d.position_ms || 0); }
      else if (d.cmd === "set_volume") { player.setVolume(Math.max(0, Math.min(1, d.volume ?? 1))); }
      else if (d.cmd === "disconnect") { lastPauseCmdAt = Date.now(); player.disconnect(); }
    }

    function probeWidevine() {
      const fail = (why) => {
        logLine("DRM MISSING: " + why);
        setStatus(LICENSE_HINT);
        send({ event: "error", kind: "widevine", message: "DRM unavailable: " + why });
      };
      if (!navigator.requestMediaKeySystemAccess) { fail("no EME support"); return; }
      const audioCaps = [{ contentType: 'audio/mp4;codecs="mp4a.40.2"' }];
      const systems = [
        ["com.widevine.alpha", [{ initDataTypes: ["cenc"], audioCapabilities: audioCaps }]],
        ["com.apple.fps", [{ initDataTypes: ["sinf"], audioCapabilities: audioCaps }, { initDataTypes: ["skd"], audioCapabilities: audioCaps }]],
        ["com.apple.fps.1_0", [{ initDataTypes: ["sinf"], audioCapabilities: audioCaps }, { initDataTypes: ["skd"], audioCapabilities: audioCaps }]],
      ];
      (function tryNext(i) {
        if (i >= systems.length) { fail("no Widevine or FairPlay"); return; }
        navigator.requestMediaKeySystemAccess(systems[i][0], systems[i][1])
          .then(() => logLine("drm OK: " + systems[i][0]))
          .catch(() => tryNext(i + 1));
      })(0);
    }

    function boot() {
      logLine("booting SDK (token " + (token ? "present" : "MISSING") + ")");
      logLine("ua: " + navigator.userAgent);
      if (IS_FIREFOX) logLine("WARNING: Firefox has a known Spotify SDK bug (web-playback-sdk#116) — if tracks fail, use Chrome/Edge/Brave");
      probeWidevine();
      probeAutoplay();
      player = new Spotify.Player({
        name: "kopuz",
        getOAuthToken: (cb) => { tokenCalls++; logLine("getOAuthToken #" + tokenCalls); cb(token); },
        volume: 1.0,
      });
      player.addListener("ready", ({ device_id }) => {
        isReady = true;
        claimMediaSession(null);
        send({ event: "ready", device_id });
        setStatus(autoplayBlocked ? "Click the button to enable playback."
                                  : "Ready — audio plays in this tab.");
        logLine("READY device=" + device_id);
      });
      player.addListener("not_ready", () => { isReady = false; send({ event: "not_ready" }); setStatus("Device went offline."); logLine("NOT_READY"); });
      player.addListener("initialization_error", ({ message }) => {
        send({ event: "error", kind: "widevine", message }); setStatus("Playback unavailable: " + message);
        logLine("INIT_ERROR (no Widevine?): " + message);
      });
      player.addListener("authentication_error", ({ message }) => {
        send({ event: "error", kind: "auth", message }); setStatus("Sign-in expired: " + message);
        logLine("AUTH_ERROR: " + message);
      });
      player.addListener("account_error", ({ message }) => {
        send({ event: "error", kind: "account", message }); setStatus("Spotify Premium is required for playback.");
        logLine("ACCOUNT_ERROR (Premium?): " + message);
      });
      player.addListener("playback_error", ({ message }) => {
        send({ event: "error", kind: "playback", message });
        logLine("PLAYBACK_ERROR: " + message);
        const now = Date.now();
        errTimes = errTimes.filter((t) => now - t < 20000);
        errTimes.push(now);
        if (hasPlayed && errTimes.length >= 3) {
          errTimes = [];
          setStatus(LICENSE_HINT);
          send({ event: "error", kind: "license", message: LICENSE_HINT });
        }
      });
      player.addListener("player_state_changed", (s) => {
        if (!s) { logLine("state=null (device lost active session)"); return; }
        const cur = s.track_window && s.track_window.current_track;
        logLine("state paused=" + s.paused + " pos=" + (s.position || 0) + " dur=" + (s.duration || 0) + " track=" + (cur ? cur.name : "null"));
        handleState(s);
      });
      player.connect().then((ok) => logLine("connect() -> " + ok));

      setInterval(async () => {
        if (!player) return;
        let st = null;
        try { st = await player.getCurrentState(); } catch (e) {}
        if (!st) { ensureMediaMetadata(null); return; }
        const pos = st.position || 0, dur = st.duration || 0;
        if (!st.paused) {
          if (pos === lastPos && Date.now() - lastPosAt > 3500 && dur > 0 && pos < dur - 2000) {
            if (stallResumes < 3) {
              stallResumes += 1;
              logLine("STALL: playing but pos stuck at " + pos + "/" + dur + " — resuming (" + stallResumes + "/3)");
              try { await player.resume(); } catch (e) {}
            } else if (stallResumes === 3) {
              stallResumes += 1;
              logLine("STALL: giving up after 3 resumes");
              setStatus(LICENSE_HINT);
              send({ event: "error", kind: "license", message: LICENSE_HINT });
            }
            lastPosAt = Date.now();
          }
          if (pos !== lastPos) { lastPos = pos; lastPosAt = Date.now(); stallResumes = 0; }
        } else {
          lastPos = pos; lastPosAt = Date.now();
          if (unexpectedPauseAt && Date.now() - unexpectedPauseAt > 3000
              && Date.now() - lastPauseCmdAt > 4000) {
            unexpectedPauseAt = 0;
            lastDeathAt = Date.now();
            if (deathRetries === 0) {
              deathRetries = 1;
              logLine("DIED mid-track at " + pos + "/" + dur + " — retrying resume");
              try { await player.resume(); } catch (e) {}
            } else {
              deathRetries += 1;
              logLine("DIED again at " + pos + "/" + dur);
              setStatus(LICENSE_HINT);
              if (deathRetries === 2) send({ event: "error", kind: "license", message: LICENSE_HINT });
            }
          }
        }
        handleState(st);
      }, 1000);
    }

    let lastSentKey = "";
    function handleState(s) {
      const paused = s.paused, pos = s.position || 0, dur = s.duration || 0;
      const cur = s.track_window && s.track_window.current_track;
      const tid = cur ? cur.id : null;

      ensureMediaMetadata(cur);
      try { if ("mediaSession" in navigator) navigator.mediaSession.playbackState = paused ? "paused" : "playing"; } catch (e) {}

      if (tid !== curId) {
        curId = tid;
        reportedEnded = false;
        nearEnd = false;
        deathRetries = 0;
        prevPaused = paused;
        wasPlaying = !paused;
        claimMediaSession(cur);
        lastSentKey = paused + "|" + pos + "|" + tid;
        send({ event: "state", paused, position: pos, duration: dur, track_id: tid, ended: false });
        return;
      }

      if (paused) {
        if (!prevPaused && pos > 0 && dur > 0 && pos < dur - 3000
            && Date.now() - lastPauseCmdAt > 4000 && !unexpectedPauseAt) {
          unexpectedPauseAt = Date.now();
        }
      } else {
        unexpectedPauseAt = 0;
        if (deathRetries > 0 && lastDeathAt && Date.now() - lastDeathAt > 30000) {
          deathRetries = 0;
          if (statusEl.textContent === LICENSE_HINT) setStatus("Ready — audio plays in this tab.");
        }
      }
      prevPaused = paused;

      if (!paused) {
        wasPlaying = true;
        reportedEnded = false;
        if (pos > 2000) hasPlayed = true;
      }
      if (!paused && dur > 0 && pos >= dur - 3000) nearEnd = true;

      let ended = false;
      if (paused && pos === 0 && wasPlaying && nearEnd && !reportedEnded) {
        ended = true;
        reportedEnded = true;
        wasPlaying = false;
        nearEnd = false;
      }
      const key = paused + "|" + pos + "|" + tid;
      if (!ended && key === lastSentKey) return;
      lastSentKey = key;
      send({ event: "state", paused, position: pos, duration: dur, track_id: tid, ended });
    }

    document.getElementById("activate").addEventListener("click", () => {
      logLine("activate clicked");
      userActivated = true;
      autoplayBlocked = false;
      if (player && player.activateElement) { try { player.activateElement(); } catch (e) {} }
      send({ event: "activated" });
      setStatus("Playback enabled.");
    });

    window.onSpotifyWebPlaybackSDKReady = () => { if (token && !player) boot(); };
    connect();
  </script>
  <script src="https://sdk.scdn.co/spotify-player.js"></script>
</body>
</html>"#;

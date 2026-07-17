//! Browser-hosted Spotify playback.
//!
//! The Web Playback SDK needs a Widevine CDM; kopuz's embedded webview lacks one
//! on macOS/Linux, so we can't run the SDK in-app. Instead the host runs a tiny
//! localhost server, opens the user's own browser (which ships Widevine) at a
//! page that boots `Spotify.Player`, and drives it over a WebSocket:
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
    /// Bind an ephemeral localhost port, start serving, and open the browser.
    pub async fn start(access: String) -> Result<Self, String> {
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
        open_player_page(&url)?;

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

/// Open the player page, preferring a Chromium-family browser. The Web Playback
/// SDK has a long-standing Firefox bug where tracks die with generic
/// `playback_error`s (spotify/web-playback-sdk#116), so the system default
/// browser is only used as a last resort.
fn open_player_page(url: &str) -> Result<(), String> {
    if webbrowser::open_browser(webbrowser::Browser::Chrome, url).is_ok() {
        tracing::info!("spotify player page opened in Chrome");
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    for bundle in [
        "com.microsoft.edgemac",
        "com.brave.Browser",
        "org.chromium.Chromium",
        "com.vivaldi.Vivaldi",
    ] {
        let opened = std::process::Command::new("open")
            .args(["-b", bundle, url])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if opened {
            tracing::info!(
                bundle,
                "spotify player page opened in Chromium-family browser"
            );
            return Ok(());
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    for cmd in [
        "chromium",
        "chromium-browser",
        "brave-browser",
        "microsoft-edge",
        "vivaldi",
    ] {
        if std::process::Command::new(cmd).arg(url).spawn().is_ok() {
            tracing::info!(cmd, "spotify player page opened in Chromium-family browser");
            return Ok(());
        }
    }
    #[cfg(target_os = "windows")]
    {
        // Edge ships with Windows; its protocol handler beats an unknown default.
        let opened = std::process::Command::new("cmd")
            .args(["/C", "start", "", &format!("microsoft-edge:{url}")])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if opened {
            tracing::info!("spotify player page opened in Edge");
            return Ok(());
        }
    }
    tracing::warn!("no Chromium-family browser found; falling back to the default browser");
    webbrowser::open(url)
        .map(|_| ())
        .map_err(|e| format!("couldn't open the browser for Spotify playback: {e}"))
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
    // Peek the request line + headers without consuming, so a WebSocket upgrade
    // can be handed to the tungstenite handshake untouched.
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
        // Consume the request we peeked, then serve.
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

    // Push the current token immediately so the SDK can boot on first connect.
    let initial = {
        let t = token.lock().await.clone();
        json!({ "cmd": "set_token", "token": t }).to_string()
    };
    if sink.send(Message::text(initial)).await.is_err() {
        return;
    }

    // Forward outgoing command frames to the browser.
    let mut cmd_rx = cmd_tx.subscribe();
    let forward = tokio::spawn(async move {
        while let Ok(msg) = cmd_rx.recv().await {
            if sink.send(Message::text(msg)).await.is_err() {
                break;
            }
        }
    });

    // Read player events from the browser.
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
    let out = match event {
        "ready" => HostEvent::Ready {
            device_id: val["device_id"].as_str().unwrap_or_default().to_string(),
        },
        "not_ready" => HostEvent::NotReady,
        "activated" => HostEvent::Activated,
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
    };
    const send = (o) => { try { if (ws && ws.readyState === 1) ws.send(JSON.stringify(o)); } catch (e) {} };

    // Log failing Spotify network calls. The SDK collapses every failure —
    // rejected DRM license, blocked CDN chunk — into a generic "Playback error",
    // so hook fetch/XHR (before the SDK script loads, below) and name the
    // endpoint that actually failed.
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

    // Detect whether audible autoplay is allowed. If it is, report "activated"
    // right away so kopuz can start playback without the button click; if not
    // (Firefox's default), kopuz holds the first track until the user clicks —
    // firing plays into an autoplay block just storms errors and can wedge the
    // SDK's playback pipeline.
    function probeAutoplay() {
      const done = (ok, why) => {
        logLine("autoplay " + (ok ? "allowed" : "blocked" + (why ? " (" + why + ")" : "")));
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

    function connect() {
      ws = new WebSocket("ws://" + location.host + "/ws");
      ws.onmessage = (m) => { let d; try { d = JSON.parse(m.data); } catch (e) { return; } onCommand(d); };
      ws.onclose = () => { setTimeout(connect, 1000); };
    }

    function onCommand(d) {
      if (d.cmd !== "set_token") logLine("cmd " + d.cmd + (d.position_ms != null ? " " + d.position_ms : ""));
      if (d.cmd === "set_token") {
        const fresh = (d.token || "") && (d.token || "") !== token;
        token = d.token || "";
        if (!player && window.Spotify) boot();
        // A rotated token while the SDK is down (expired token killed the last
        // connect) is the recovery path: reconnect with the fresh one.
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

    // The SDK boots fine in browsers whose EME isn't Widevine (e.g. Safari uses
    // FairPlay) and then fails every license request, killing each track ~10s in
    // with only generic playback errors. Probe for Widevine directly so that
    // failure mode is named the moment the page loads.
    function probeWidevine() {
      const fail = (why) => {
        logLine("WIDEVINE MISSING: " + why);
        setStatus(LICENSE_HINT);
        send({ event: "error", kind: "widevine", message: "Widevine unavailable: " + why });
      };
      if (!navigator.requestMediaKeySystemAccess) { fail("no EME support"); return; }
      navigator.requestMediaKeySystemAccess("com.widevine.alpha", [{
        initDataTypes: ["cenc"],
        audioCapabilities: [{ contentType: 'audio/mp4;codecs="mp4a.40.2"' }],
      }]).then(() => logLine("widevine OK"))
        .catch((e) => fail(e && e.message ? e.message : "unavailable"));
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
        // A burst of generic errors after real playback has happened is the
        // license-failure signature (pre-play bursts are just autoplay blocks).
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

      // The SDK only fires player_state_changed on transitions, never while a
      // track simply plays — so poll getCurrentState() for the between-event
      // position ticks kopuz needs (progress bar + end-of-track detection).
      // The same poll doubles as a stall watchdog: if audio stalls (e.g. a
      // Widevine license failure) the SDK goes silent, so if position isn't
      // advancing while "playing", try to resume once and surface it.
      setInterval(async () => {
        if (!player) return;
        let st = null;
        try { st = await player.getCurrentState(); } catch (e) {}
        if (!st) { return; }
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
        }
        handleState(st);
      }, 1000);
    }

    let lastSentKey = "";
    function handleState(s) {
      const paused = s.paused, pos = s.position || 0, dur = s.duration || 0;
      const cur = s.track_window && s.track_window.current_track;
      const tid = cur ? cur.id : null;

      // A different track loaded (kopuz issued the next URI). A fresh load also
      // reports paused@0, so reset per-track state and never treat it as an end,
      // or every track change would auto-skip.
      if (tid !== curId) {
        curId = tid;
        reportedEnded = false;
        nearEnd = false;
        deathRetries = 0;
        prevPaused = paused;
        wasPlaying = !paused;
        lastSentKey = paused + "|" + pos + "|" + tid;
        send({ event: "state", paused, position: pos, duration: dur, track_id: tid, ended: false });
        return;
      }

      // The SDK flipping to paused mid-track without kopuz asking is playback
      // dying underneath us (typically a failed DRM license ~10s in). Retry once
      // per track, then surface the diagnosis instead of stopping silently.
      if (paused && !prevPaused && pos > 0 && dur > 0 && pos < dur - 3000
          && Date.now() - lastPauseCmdAt > 4000) {
        if (deathRetries === 0) {
          deathRetries = 1;
          logLine("DIED mid-track at " + pos + "/" + dur + " — retrying resume");
          try { player.resume().catch(() => {}); } catch (e) {}
        } else {
          deathRetries += 1;
          logLine("DIED again at " + pos + "/" + dur);
          setStatus(LICENSE_HINT);
          if (deathRetries === 2) send({ event: "error", kind: "license", message: LICENSE_HINT });
        }
      }
      prevPaused = paused;

      if (!paused) {
        wasPlaying = true;
        reportedEnded = false;
        if (pos > 2000) hasPlayed = true;
      }
      // Latch once playback actually reaches the tail of the track. Only then can
      // a following paused@0 be a genuine end — a mid-track network stall (which
      // also reports paused, sometimes at 0) never crosses this line, so it can't
      // be mistaken for the song finishing.
      if (!paused && dur > 0 && pos >= dur - 3000) nearEnd = true;

      let ended = false;
      if (paused && pos === 0 && wasPlaying && nearEnd && !reportedEnded) {
        ended = true;
        reportedEnded = true;
        wasPlaying = false;
        nearEnd = false;
      }
      // Polls repeat the same frame while paused — only forward actual changes.
      const key = paused + "|" + pos + "|" + tid;
      if (!ended && key === lastSentKey) return;
      lastSentKey = key;
      send({ event: "state", paused, position: pos, duration: dur, track_id: tid, ended });
    }

    document.getElementById("activate").addEventListener("click", () => {
      logLine("activate clicked");
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

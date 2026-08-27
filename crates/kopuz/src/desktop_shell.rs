#[cfg(not(target_os = "android"))]
pub fn build_window_icon() -> Option<dioxus::desktop::tao::window::Icon> {
    let image = image::load_from_memory(include_bytes!("../assets/logo-512.png")).ok()?;
    let image = image.into_rgba8();
    let (width, height) = image.dimensions();
    dioxus::desktop::tao::window::Icon::from_rgba(image.into_raw(), width, height).ok()
}

#[cfg(not(target_os = "android"))]
pub fn build_tray_icon() -> Option<dioxus::desktop::trayicon::Icon> {
    let image = image::load_from_memory(include_bytes!("../assets/logo-512.png")).ok()?;
    let image = image.into_rgba8();
    let (width, height) = image.dimensions();
    dioxus::desktop::trayicon::Icon::from_rgba(image.into_raw(), width, height).ok()
}

#[cfg(target_os = "linux")]
pub fn tray_backend_available() -> bool {
    const CANDIDATES: &[&str] = &[
        "libayatana-appindicator3.so.1",
        "libappindicator3.so.1",
        "libayatana-appindicator3.so",
        "libappindicator3.so",
    ];
    CANDIDATES
        .iter()
        .any(|name| unsafe { libloading::Library::new(*name) }.is_ok())
}

#[cfg(all(not(target_os = "android"), not(target_os = "linux")))]
pub fn tray_backend_available() -> bool {
    true
}

#[cfg(not(target_os = "android"))]
pub fn show_tray_missing_popup() {
    let msg = "System tray unavailable: appindicator library not found. \
               Install libayatana-appindicator (Debian/Ubuntu/Arch) or \
               libappindicator-gtk3 (Fedora). Closing the window will quit \
               the app instead of minimizing to tray.";
    let escaped = serde_json::to_string(msg).unwrap_or_else(|_| "\"\"".to_string());
    let js = format!(
        r#"(function(m){{
            let t = document.getElementById('kopuz-tray-popup');
            if (!t) {{
                t = document.createElement('div');
                t.id = 'kopuz-tray-popup';
                t.style.cssText = 'position:fixed;right:16px;top:16px;max-width:360px;background:rgba(28,28,30,0.97);color:#fff;padding:14px 16px;border-radius:10px;font:13px/1.45 system-ui,sans-serif;z-index:99999;box-shadow:0 8px 28px rgba(0,0,0,0.5);border:1px solid rgba(255,170,60,0.45);opacity:0;transition:opacity 200ms;';
                t.onclick = () => {{ t.style.opacity = '0'; }};
                document.body.appendChild(t);
            }}
            t.innerHTML = '<div style="font-weight:600;margin-bottom:4px;color:#ffb347;">Tray icon unavailable</div>' + m;
            requestAnimationFrame(() => {{ t.style.opacity = '1'; }});
            clearTimeout(t._h);
            t._h = setTimeout(() => {{ t.style.opacity = '0'; }}, 8000);
        }})({escaped});"#
    );
    let _ = dioxus::document::eval(&js);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn read_titlebar_mode_from_disk() -> config::TitlebarMode {
    db::peek_config(&db::default_db_path())
        .map(|c| c.titlebar_mode)
        .unwrap_or_default()
}

/// Head script that takes the webview's frame clock out of the VirtualDom's
/// polling path.
///
/// dioxus-desktop sends every render as a batch over the edits websocket and
/// refuses to poll the VirtualDom again until the page acks it
/// (`WryQueue::poll_edits_flushed`, an early `return` in `WebviewInstance::poll_vdom`).
/// The interpreter sends that ack from inside a `requestAnimationFrame`
/// callback, so the ack only lands when the page is being composited. A Wayland
/// compositor stops sending frame callbacks to an unfocused or occluded
/// surface, and WebKitGTK suspends rAF with them; minimised windows do the same
/// on macOS and Windows. The wakeups still arrive (engine events, tokio timers),
/// but every one of them hits the edits gate and returns, so no Rust-side task
/// runs until the window comes back and the backlog fires at once. That is why a
/// track that ends while the window is in the background only advances on focus.
///
/// Running the batch straight off the socket keeps the ack on the websocket's
/// own clock. The webview still paints on whatever schedule the compositor
/// gives it; only the ack stops waiting for a frame.
#[cfg(not(target_os = "android"))]
pub const UNGATE_EDITS_FROM_FRAME_CLOCK: &str = r#"<script>
(function () {
  var attempts = 0;
  function patch() {
    var interpreter = window.interpreter;
    if (!interpreter) {
      attempts += 1;
      if (attempts > 600) {
        console.warn('kopuz: interpreter never appeared; edits are still gated on requestAnimationFrame');
        return;
      }
      setTimeout(patch, 50);
      return;
    }
    if (typeof interpreter.run_from_bytes !== 'function'
      || typeof interpreter.markEditsFinished !== 'function') {
      console.warn('kopuz: dioxus interpreter changed shape; edits are still gated on requestAnimationFrame');
      return;
    }
    interpreter.rafEdits = function (bytes) {
      this.run_from_bytes(bytes);
      this.markEditsFinished();
    };
  }
  patch();
})();
</script>"#;

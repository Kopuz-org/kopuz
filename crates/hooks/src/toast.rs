//! A tiny transient toast, rendered by injecting a fixed-position element via
//! `document.eval` (no component/state plumbing needed for a fire-and-forget
//! message). Shared so any hook or component can surface a brief notice.

/// Show `msg` as a neutral (info) toast for ~1.8s.
pub fn toast(msg: &str) {
    show(msg, "rgba(20,20,20,0.95)", "rgba(255,255,255,0.1)");
}

/// Show `msg` as an error toast (reddish) — for a failure the user should notice.
pub fn toast_error(msg: &str) {
    show(msg, "rgba(45,18,18,0.96)", "rgba(239,68,68,0.55)");
}

/// Bottom-centered toast reusing a single `#kopuz-toast` element; `bg`/`border`
/// are re-applied every call so the level (info vs error) always matches `msg`.
fn show(msg: &str, bg: &str, border: &str) {
    let escaped = serde_json::to_string(msg).unwrap_or_else(|_| "\"\"".to_string());
    let js = format!(
        r#"(function(m){{
            let t = document.getElementById('kopuz-toast');
            if (!t) {{
                t = document.createElement('div');
                t.id = 'kopuz-toast';
                t.style.cssText = 'position:fixed;left:50%;bottom:88px;transform:translateX(-50%);color:#fff;padding:10px 18px;border-radius:8px;font:14px system-ui,sans-serif;z-index:99999;box-shadow:0 4px 16px rgba(0,0,0,0.4);pointer-events:none;opacity:0;transition:opacity 150ms;border:1px solid transparent;';
                document.body.appendChild(t);
            }}
            t.style.background = '{bg}';
            t.style.borderColor = '{border}';
            t.textContent = m;
            t.style.opacity = '1';
            clearTimeout(t._h);
            t._h = setTimeout(() => {{ t.style.opacity = '0'; }}, 1800);
        }})({escaped});"#
    );
    let _ = dioxus::document::eval(&js);
}

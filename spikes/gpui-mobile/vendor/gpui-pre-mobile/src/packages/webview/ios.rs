use super::WebViewHandle;

#[link(name = "WebKit", kind = "framework")]
extern "C" {}

pub fn evaluate_javascript(handle: &WebViewHandle, script: &str) -> Result<(), String> {
    // The platform view system handles the WKWebView lifecycle.
    // For JS evaluation, we need access to the underlying WKWebView.
    // This is currently a no-op — the platform view manages the webview.
    // TODO: Store a reference to the WKWebView for runtime JS evaluation.
    let _ = (handle, script);
    log::warn!("evaluate_javascript: not yet wired through platform view system");
    Ok(())
}

pub fn go_back(handle: &WebViewHandle) -> Result<(), String> {
    let _ = handle;
    log::warn!("go_back: not yet wired through platform view system");
    Ok(())
}

pub fn reload(handle: &WebViewHandle) -> Result<(), String> {
    let _ = handle;
    log::warn!("reload: not yet wired through platform view system");
    Ok(())
}

pub fn stop_loading(handle: &WebViewHandle) -> Result<(), String> {
    let _ = handle;
    log::warn!("stop_loading: not yet wired through platform view system");
    Ok(())
}

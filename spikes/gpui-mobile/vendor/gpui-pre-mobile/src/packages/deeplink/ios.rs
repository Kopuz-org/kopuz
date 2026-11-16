use std::sync::Mutex;

static INITIAL_LINK: Mutex<Option<String>> = Mutex::new(None);
static LATEST_LINK: Mutex<Option<String>> = Mutex::new(None);

pub fn get_initial_link() -> Result<Option<String>, String> {
    Ok(INITIAL_LINK.lock().unwrap().clone())
}

pub fn get_latest_link() -> Option<String> {
    LATEST_LINK.lock().unwrap().clone()
}

/// Called from the FFI layer when a URL is opened.
pub(crate) fn handle_open_url(url: String) {
    let is_initial = INITIAL_LINK.lock().unwrap().is_none();
    if is_initial {
        *INITIAL_LINK.lock().unwrap() = Some(url.clone());
    }
    *LATEST_LINK.lock().unwrap() = Some(url.clone());
    super::notify_deep_link(&url);
}

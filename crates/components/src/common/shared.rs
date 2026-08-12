#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayoutMode {
    Rightbar,
    Fullscreen,
}

impl std::fmt::Display for LayoutMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutMode::Rightbar => write!(f, "rightbar"),
            LayoutMode::Fullscreen => write!(f, "fullscreen"),
        }
    }
}

pub fn fmt_time(secs: u64) -> String {
    if secs == u64::MAX {
        return "--:--".to_string();
    }
    let m = secs / 60;
    let s = secs % 60;
    format!("{m}:{s:02}")
}

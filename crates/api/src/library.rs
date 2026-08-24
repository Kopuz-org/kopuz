use serde::{Deserialize, Serialize};

pub const DEFAULT_PAGE_LIMIT: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    pub offset: u32,
    pub limit: u32,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: DEFAULT_PAGE_LIMIT,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
}

/// A window into a filtered track listing. `total` always reflects the whole
/// filtered set so clients can paginate without a second count request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackPage {
    pub total: u32,
    pub offset: u32,
    pub items: Vec<reader::Track>,
}

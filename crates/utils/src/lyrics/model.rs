use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricChunk {
    /// A timed lyric chunk. Most providers use whole words; Apple Music can
    /// return smaller syllable-level chunks.
    pub start_time: f64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricLine {
    pub start_time: f64,
    #[serde(default)]
    pub end_time: Option<f64>,
    pub text: String,
    #[serde(default)]
    pub chunks: Vec<LyricChunk>,
    #[serde(default)]
    pub parent_line_index: Option<usize>,
    #[serde(default)]
    pub background: bool,
    #[serde(default)]
    pub opposite_turn: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lyrics {
    Synced(Vec<LyricLine>),
    Plain(String),
}

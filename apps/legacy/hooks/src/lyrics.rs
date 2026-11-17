//! The words to what is playing.
//!
//! Finding them means asking several providers, and for some services signing
//! the request with the account's own token, so the daemon does it: one call
//! per track, answered from its cache when it has one.

use config::{LyricsEdit, SettingsEdit};
use dioxus::prelude::*;
use utils::lyrics::{LyricChunk, LyricLine, Lyrics};

use crate::api::use_api;

/// The words, and the romanized reading the daemon gave each line: one per
/// synced line, or per line of plain text; empty when none has one.
#[derive(Debug, Clone, PartialEq)]
pub struct ShownLyrics {
    pub lyrics: Lyrics,
    pub readings: Vec<Option<String>>,
}

impl ShownLyrics {
    fn plain(text: String) -> Self {
        Self {
            lyrics: Lyrics::Plain(text),
            readings: Vec::new(),
        }
    }
}

/// `None` while the lookup is running, `Some(None)` when there are none.
pub type LyricsState = Option<Option<ShownLyrics>>;

/// The lyrics for a track key, re-read when the track changes. `not_found`
/// is what a view shows when there are none: the string table is the
/// caller's, so the text comes with the ask. A radio stream
/// is answered immediately: station names only ever match junk.
///
/// The daemon's chain runs to its slowest provider, so a track skipped past
/// seconds ago still answers, and answers after the track that replaced it.
/// An answer is taken only while its own key is still the one playing.
///
/// The daemon romanizes, so the words are asked for again once a change to
/// that setting has reached it.
pub fn use_lyrics(key: Memo<String>, radio: Memo<bool>, not_found: String) -> Signal<LyricsState> {
    let api = use_api();
    let settings = crate::settings::use_settings();
    let mut state = use_signal(|| None as LyricsState);
    let mut asked_for = use_signal(|| None::<(String, bool)>);
    let romanize = use_memo(move || {
        let saving = settings
            .unsent_tracked()
            .iter()
            .any(|edit| matches!(edit, SettingsEdit::Lyrics(LyricsEdit::RomanizeLyrics(_))));
        (!saving).then(|| settings.settings().read().lyrics.romanize_lyrics)
    });
    use_effect(move || {
        let asked = key();
        if asked.is_empty() || radio() {
            asked_for.set(None);
            state.set(Some(None));
            return;
        }
        // Until the edit is saved the daemon would answer under the old setting.
        let Some(romanize) = romanize() else {
            return;
        };
        let previous = asked_for.peek().clone();
        if previous.as_ref() == Some(&(asked.clone(), romanize)) {
            return;
        }
        // The same track asked again keeps its words on show until the new ones come.
        if previous.is_none_or(|(key, _)| key != asked) {
            state.set(None);
        }
        asked_for.set(Some((asked.clone(), romanize)));
        let api = api.clone();
        let not_found = not_found.clone();
        spawn(async move {
            let found = match api.lyrics(asked.clone()).await {
                Ok(view) => Some(from_view(view)),
                Err(error) => {
                    tracing::debug!(%error, key = %asked, "no lyrics for the playing track");
                    None
                }
            };
            if asked_for.peek().as_ref() != Some(&(asked.clone(), romanize)) {
                return;
            }
            let found = found.or_else(|| Some(ShownLyrics::plain(not_found.clone())));
            state.set(Some(found));
        });
    });
    state
}

/// The wire's lyrics as the views render them.
fn from_view(view: api::LyricsView) -> ShownLyrics {
    if let Some(plain) = view.plain {
        return ShownLyrics {
            lyrics: Lyrics::Plain(plain),
            readings: view.plain_romanized,
        };
    }
    let seconds = |ms: u64| ms as f64 / 1000.0;
    let readings: Vec<Option<String>> = view
        .synced
        .iter()
        .map(|line| line.romanized.clone())
        .collect();
    ShownLyrics {
        readings: if readings.iter().any(Option::is_some) {
            readings
        } else {
            Vec::new()
        },
        lyrics: Lyrics::Synced(
            view.synced
                .into_iter()
                .map(|line| LyricLine {
                    start_time: seconds(line.start_ms),
                    end_time: line.end_ms.map(seconds),
                    text: line.text,
                    chunks: line
                        .chunks
                        .into_iter()
                        .map(|chunk| LyricChunk {
                            start_time: seconds(chunk.start_ms),
                            text: chunk.text,
                        })
                        .collect(),
                    parent_line_index: line.parent_line_index.map(|index| index as usize),
                    background: line.background,
                    opposite_turn: line.opposite_turn,
                })
                .collect(),
        ),
    }
}

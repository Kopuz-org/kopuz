//! The words to what is playing.
//!
//! Finding them means asking several providers, and for some services signing
//! the request with the account's own token, so the daemon does it: one call
//! per track, answered from its cache when it has one.

use dioxus::prelude::*;
use utils::lyrics::{LyricChunk, LyricLine, Lyrics};

use crate::api::use_api;

/// `None` while the lookup is running, `Some(None)` when there are none.
pub type LyricsState = Option<Option<Lyrics>>;

/// The lyrics for a track key, re-read when the track changes. `not_found`
/// is what a view shows when there are none: the string table is the
/// caller's, so the text comes with the ask. A radio stream
/// is answered immediately: station names only ever match junk.
///
/// The daemon's chain runs to its slowest provider, so a track skipped past
/// seconds ago still answers, and answers after the track that replaced it.
/// An answer is taken only while its own key is still the one playing.
pub fn use_lyrics(key: Memo<String>, radio: Memo<bool>, not_found: String) -> Signal<LyricsState> {
    let api = use_api();
    let mut state = use_signal(|| None as LyricsState);
    use_effect(move || {
        let asked = key();
        if asked.is_empty() || radio() {
            state.set(Some(None));
            return;
        }
        state.set(None);
        let api = api.clone();
        let not_found = not_found.clone();
        spawn(async move {
            let found = api.lyrics(asked.clone()).await.ok().map(from_view);
            if *key.peek() != asked {
                return;
            }
            let found = found.or_else(|| Some(Lyrics::Plain(not_found.clone())));
            state.set(Some(found));
        });
    });
    state
}

/// The wire's lyrics as the views render them.
fn from_view(view: api::LyricsView) -> Lyrics {
    if let Some(plain) = view.plain {
        return Lyrics::Plain(plain);
    }
    let seconds = |ms: u64| ms as f64 / 1000.0;
    Lyrics::Synced(
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
    )
}

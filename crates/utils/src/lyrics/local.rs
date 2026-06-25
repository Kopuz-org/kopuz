use super::{Lyrics, has_usable_line_timing, lrc::parse_lrc, lrc_has_usable_timing};

#[cfg(not(target_arch = "wasm32"))]
pub(super) async fn fetch_local_lrc(audio_path: &str) -> Option<Lyrics> {
    let content = read_local_lrc(audio_path).or_else(|| read_embedded_lyrics(audio_path))?;
    if content.trim().is_empty() {
        return None;
    }
    let lines = if lrc_has_usable_timing(&content) {
        parse_lrc(&content)
    } else {
        Vec::new()
    };
    if has_usable_line_timing(&lines) {
        Some(Lyrics::Synced(lines))
    } else {
        Some(Lyrics::Plain(content))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_embedded_lyrics(audio_path: &str) -> Option<String> {
    use lofty::file::TaggedFileExt;
    use lofty::probe::Probe;
    use lofty::tag::ItemKey;

    let tagged = Probe::open(audio_path).ok()?.read().ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    tag.get_string(ItemKey::Lyrics)
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_local_lrc(audio_path: &str) -> Option<String> {
    use std::path::Path;

    let audio = Path::new(audio_path);

    let stem_lrc = audio.with_extension("lrc");
    if let Ok(content) = std::fs::read_to_string(&stem_lrc) {
        return Some(content);
    }

    let appended = format!("{audio_path}.lrc");
    if let Ok(content) = std::fs::read_to_string(&appended) {
        return Some(content);
    }

    let parent = audio.parent()?;
    let file_name = audio.file_name()?.to_string_lossy().to_lowercase();
    let stem = audio
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase());

    for entry in std::fs::read_dir(parent).ok()?.flatten() {
        let path = entry.path();
        if path
            .extension()
            .map(|e| !e.eq_ignore_ascii_case("lrc"))
            .unwrap_or(true)
        {
            continue;
        }
        let Some(cand_name) = path.file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
            continue;
        };
        let matches_appended = cand_name == format!("{file_name}.lrc");
        let matches_stem = stem
            .as_ref()
            .map(|s| cand_name == format!("{s}.lrc"))
            .unwrap_or(false);
        if (matches_appended || matches_stem)
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            return Some(content);
        }
    }

    None
}

#[cfg(target_arch = "wasm32")]
pub(super) async fn fetch_local_lrc(_audio_path: &str) -> Option<Lyrics> {
    None
}

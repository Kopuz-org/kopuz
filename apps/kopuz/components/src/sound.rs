//! What something sounds like: its tags and standout moods as one line of badges.

use api::SoundView;
use gpui_kit::component::h_flex;
use gpui_kit::{AnyElement, IntoElement, ParentElement as _, SharedString, Styled as _};

use crate::chip::{Badge, BadgeTone, ChipSize};
use crate::format::t;

/// `None` when AudioMuse-AI gave neither.
pub fn sound_profile(sound: &SoundView) -> Option<AnyElement> {
    if sound.tags.is_empty() && sound.moods.is_empty() {
        return None;
    }
    let moods = sound.moods.iter().filter_map(|mood| {
        let key = mood_key(mood)?;
        Some(
            Badge::new(t(key))
                .tone(BadgeTone::Accent)
                .size(ChipSize::Medium),
        )
    });
    let tags = sound.tags.iter().map(|tag| {
        Badge::new(SharedString::from(tag.clone()))
            .tone(BadgeTone::Neutral)
            .size(ChipSize::Medium)
    });
    Some(
        h_flex()
            .w_full()
            .flex_wrap()
            .gap_2()
            .children(moods)
            .children(tags)
            .into_any_element(),
    )
}

/// The i18n key of a mood the analysis names; any other is not shown.
fn mood_key(mood: &str) -> Option<&'static str> {
    Some(match mood {
        "danceable" => "mood_danceable",
        "aggressive" => "mood_aggressive",
        "happy" => "mood_happy",
        "party" => "mood_party",
        "relaxed" => "mood_relaxed",
        "sad" => "mood_sad",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_six_moods_have_a_label() {
        for mood in [
            "danceable",
            "aggressive",
            "happy",
            "party",
            "relaxed",
            "sad",
        ] {
            assert!(mood_key(mood).is_some(), "{mood}");
        }
        assert_eq!(mood_key("electronic"), None);
    }
}

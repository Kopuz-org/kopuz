//! Internationalization engine for Kopuz: loads Fluent (.ftl) locale files,
//! provides a global `t()` macro for translatable UI strings.

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use std::sync::{OnceLock, RwLock};
use unic_langid::LanguageIdentifier;

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

pub use self::AVAILABLE_LANGUAGES as LANGUAGES;

struct I18nState {
    lang: String,
    bundle: FluentBundle<FluentResource>,
    fallback: Option<FluentBundle<FluentResource>>,
}

impl I18nState {
    fn new(lang: &str) -> Self {
        let bundle = make_bundle(lang)
            .or_else(|| make_bundle("en"))
            .expect("English locale must exist");
        let fallback = if lang != "en" {
            make_bundle("en")
        } else {
            None
        };
        I18nState {
            lang: lang.to_string(),
            bundle,
            fallback,
        }
    }

    fn translate(&self, key: &str, args: Option<&FluentArgs<'_>>) -> String {
        if let Some(result) = format_message(&self.bundle, key, args) {
            return result;
        }
        if let Some(fb) = &self.fallback
            && let Some(result) = format_message(fb, key, args)
        {
            return result;
        }
        key.to_string()
    }
}

fn format_message(
    bundle: &FluentBundle<FluentResource>,
    key: &str,
    args: Option<&FluentArgs<'_>>,
) -> Option<String> {
    let msg = bundle.get_message(key)?;
    let pattern = msg.value()?;
    let mut errors = vec![];
    Some(
        bundle
            .format_pattern(pattern, args, &mut errors)
            .to_string(),
    )
}

fn make_bundle(lang: &str) -> Option<FluentBundle<FluentResource>> {
    let content = get_ftl_content(lang)?;
    let lang_id: LanguageIdentifier = lang.parse().ok()?;
    let mut bundle = FluentBundle::new_concurrent(vec![lang_id]);
    let resource = match FluentResource::try_new(content.to_string()) {
        Ok(r) => r,
        Err((r, _)) => r,
    };
    let _ = bundle.add_resource(resource);
    Some(bundle)
}

static I18N: OnceLock<RwLock<I18nState>> = OnceLock::new();

fn state() -> &'static RwLock<I18nState> {
    I18N.get_or_init(|| RwLock::new(I18nState::new("en")))
}

pub fn init(lang: &str) {
    if I18N.set(RwLock::new(I18nState::new(lang))).is_err() {
        set_locale(lang);
    }
}

/// Switch to `lang`; the bundles are only rebuilt when it differs from the current one.
pub fn set_locale(lang: &str) {
    let mut w = state().write().unwrap_or_else(|e| e.into_inner());
    if w.lang != lang {
        *w = I18nState::new(lang);
    }
}

pub fn t(key: &str) -> String {
    state()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .translate(key, None)
}

/// Formats `key` with named arguments; pass a count as a number (`n.into()`) so its plural variants select.
pub fn t_with(key: &str, args: &[(&str, FluentValue<'_>)]) -> String {
    let mut fluent_args = FluentArgs::new();
    for (name, value) in args {
        fluent_args.set(*name, value.clone());
    }
    state()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .translate(key, Some(&fluent_args))
}

pub fn available_languages() -> &'static [(&'static str, &'static str)] {
    AVAILABLE_LANGUAGES
}

pub fn is_rtl() -> bool {
    t("is_rtl") == "true"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: String) -> String {
        text.replace(['\u{2068}', '\u{2069}'], "")
    }

    #[test]
    fn a_count_selects_the_plural_variant() {
        let count = |n: usize| plain(t_with("track_count", &[("count", n.into())]));
        assert_eq!(count(1), "1 track");
        assert_eq!(count(2), "2 tracks");
    }

    #[test]
    fn a_string_argument_is_passed_as_text() {
        let text = plain(t_with("artist_load_failed", &[("error", "gone".into())]));
        assert_eq!(text, "Couldn't load this artist: gone");
    }

    #[test]
    fn a_count_follows_the_locale_plural_rules() {
        let ru = I18nState::new("ru");
        let format = |key: &str, count: usize| {
            let mut args = FluentArgs::new();
            args.set("count", FluentValue::from(count));
            plain(ru.translate(key, Some(&args)))
        };
        assert_eq!(format("showcase_song_count", 1), "1 песня");
        assert_eq!(format("showcase_song_count", 3), "3 песни");
        assert_eq!(format("showcase_song_count", 5), "5 песен");
        assert_eq!(format("track_count", 21), "21 трек");
        assert_eq!(format("track_count", 22), "22 трека");
        assert_eq!(format("track_count", 25), "25 треков");
        assert_eq!(format("playlist_folder_count", 5), "5 плейлистов");
    }

    #[test]
    fn every_locale_parses_without_errors() {
        for (code, _) in AVAILABLE_LANGUAGES {
            let content = get_ftl_content(code).expect("locale content");
            if let Err((_, errors)) = FluentResource::try_new(content.to_string()) {
                panic!("{code}: {errors:?}");
            }
        }
    }
}

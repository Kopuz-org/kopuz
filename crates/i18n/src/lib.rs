use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use std::borrow::Cow;
use std::sync::{OnceLock, RwLock};
use unic_langid::LanguageIdentifier;

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

pub use self::AVAILABLE_LANGUAGES as LANGUAGES;

struct I18nState {
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
        I18nState { bundle, fallback }
    }

    fn translate(&self, key: &str, args: Option<&FluentArgs<'_>>) -> String {
        if let Some(result) = format_message(&self.bundle, key, args) {
            return result;
        }
        if let Some(fb) = &self.fallback {
            if let Some(result) = format_message(fb, key, args) {
                return result;
            }
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

pub fn set_locale(lang: &str) {
    *state().write().unwrap() = I18nState::new(lang);
}

pub fn t(key: &str) -> String {
    state().read().unwrap().translate(key, None)
}

pub fn t_with(key: &str, args: &[(&str, String)]) -> String {
    let mut fluent_args: FluentArgs<'static> = FluentArgs::new();
    for (k, v) in args {
        fluent_args.set(
            Cow::Owned(k.to_string()),
            FluentValue::String(Cow::Owned(v.clone())),
        );
    }
    state().read().unwrap().translate(key, Some(&fluent_args))
}

pub fn available_languages() -> &'static [(&'static str, &'static str)] {
    AVAILABLE_LANGUAGES
}

pub fn is_rtl() -> bool {
    t("is_rtl") == "true"
}

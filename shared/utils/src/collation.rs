//! The order every listing sorts text by: the system locale's ICU collation, shared so a frontend sorting a page sorts like the daemon.

use std::cmp::Ordering;
use std::sync::OnceLock;

use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};
use icu_locale_core::Locale;

/// Listing order: surrounding space ignored, then the system locale's collation.
pub fn compare(a: &str, b: &str) -> Ordering {
    static COLLATOR: OnceLock<CollatorBorrowed<'static>> = OnceLock::new();
    COLLATOR
        .get_or_init(|| collator_for(system_locale().as_deref()))
        .compare(a.trim(), b.trim())
}

/// The collator for a BCP 47 or POSIX locale name, falling back to the root order.
fn collator_for(name: Option<&str>) -> CollatorBorrowed<'static> {
    let prefs = name
        .and_then(parse_locale)
        .map(|locale| CollatorPreferences::from(&locale))
        .unwrap_or_default();
    Collator::try_new(prefs, CollatorOptions::default())
        .or_else(|_| Collator::try_new(CollatorPreferences::default(), CollatorOptions::default()))
        .expect("the compiled root collation data always loads")
}

/// `tr_TR.UTF-8@euro` and `tr-TR` both mean Turkish; `C` and `POSIX` mean no locale.
fn parse_locale(name: &str) -> Option<Locale> {
    let tag = name.split(['.', '@']).next()?.replace('_', "-");
    if tag.is_empty() || tag == "C" || tag == "POSIX" {
        return None;
    }
    tag.parse().ok()
}

#[cfg(not(any(target_os = "macos", windows, target_os = "android")))]
fn system_locale() -> Option<String> {
    ["LC_ALL", "LC_COLLATE", "LANG"]
        .into_iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|value| !value.is_empty())
}

#[cfg(target_os = "macos")]
fn system_locale() -> Option<String> {
    Some(
        objc2_foundation::NSLocale::currentLocale()
            .localeIdentifier()
            .to_string(),
    )
}

#[cfg(windows)]
fn system_locale() -> Option<String> {
    let mut name = [0u16; 85];
    // SAFETY: the buffer is LOCALE_NAME_MAX_LENGTH wide and the call writes at most that many units.
    let written = unsafe { windows::Win32::Globalization::GetUserDefaultLocaleName(&mut name) };
    let len = usize::try_from(written).ok()?.checked_sub(1)?;
    String::from_utf16(&name[..len]).ok()
}

#[cfg(target_os = "android")]
fn system_locale() -> Option<String> {
    use jni::objects::JString;

    let ctx = ndk_context::android_context();
    // SAFETY: ndk_context's VM pointer is set from the Activity before `main` and lives for the process.
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let locale = env
        .call_static_method(
            "java/util/Locale",
            "getDefault",
            "()Ljava/util/Locale;",
            &[],
        )
        .ok()?
        .l()
        .ok()?;
    let tag = env
        .call_method(&locale, "toLanguageTag", "()Ljava/lang/String;", &[])
        .ok()?
        .l()
        .ok()?;
    Some(env.get_string(&JString::from(tag)).ok()?.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(locale: &str, words: &[&'static str]) -> Vec<&'static str> {
        let collator = collator_for(Some(locale));
        let mut words = words.to_vec();
        words.sort_by(|a, b| collator.compare(a, b));
        words
    }

    #[test]
    fn accents_sort_beside_their_letter() {
        assert_eq!(
            sorted("en_US.UTF-8", &["zebra", "élan", "emu", "elan"]),
            ["elan", "élan", "emu", "zebra"]
        );
    }

    #[test]
    fn turkish_orders_dotless_i_first() {
        assert_eq!(
            sorted("tr_TR.UTF-8", &["i", "ı", "j", "h"]),
            ["h", "ı", "i", "j"]
        );
        assert_eq!(sorted("en", &["i", "ı", "j", "h"]), ["h", "i", "ı", "j"]);
    }

    #[test]
    fn posix_and_garbage_fall_back_to_the_root_order() {
        assert_eq!(parse_locale("C"), None);
        assert_eq!(parse_locale("POSIX"), None);
        assert_eq!(parse_locale("tr_TR.UTF-8@euro"), "tr-TR".parse().ok());
        assert_eq!(sorted("not a locale!", &["b", "a"]), ["a", "b"]);
    }

    #[test]
    fn surrounding_space_is_ignored() {
        assert_eq!(compare(" apple", "apple"), Ordering::Equal);
    }
}

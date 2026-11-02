//! The welcome: what Kopuz is, the language to see it in, and a word for desktop-app users.

use components::format::t;
use components::{Choice, SettingRow};
use config::{SettingsEdit, UiEdit};
use gpui_kit::component::v_flex;
use gpui_kit::{AnyElement, Context, IntoElement, ParentElement as _, SharedString, Styled};
use store::ConfigModel;

use crate::{FirstRun, flow};

impl FirstRun {
    /// Once per run, the language the session asks for, unless one was already chosen.
    pub(crate) fn offer_language(&mut self, cx: &mut Context<Self>) {
        if self.language_offered || self.step != flow::Step::Welcome {
            return;
        }
        let config = ConfigModel::global(cx);
        if !config.read(cx).is_loaded() {
            return;
        }
        self.language_offered = true;
        let current = config.read(cx).config().ui.language.clone();
        let codes: Vec<&str> = i18n::available_languages()
            .iter()
            .map(|(code, _)| *code)
            .collect();
        let wanted = flow::session_locale().and_then(|locale| flow::language_for(&locale, &codes));
        if let Some(wanted) = wanted.filter(|wanted| *wanted != current && current == "en") {
            ConfigModel::edit(cx, SettingsEdit::Ui(UiEdit::Language(wanted)));
        }
    }

    pub(crate) fn welcome_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let current = ConfigModel::global(cx)
            .read(cx)
            .config()
            .ui
            .language
            .clone();
        let options = i18n::available_languages()
            .iter()
            .map(|(code, name)| (code.to_string(), SharedString::from(*name)))
            .collect();
        let language = Choice::new("onboarding-language", current, options).on_change(
            ConfigModel::writer(|value| SettingsEdit::Ui(UiEdit::Language(value))),
        );
        v_flex()
            .w_full()
            .gap_4()
            .child(
                crate::ui::card(cx).child(
                    SettingRow::new(t("language"))
                        .setting(language)
                        .bind("language", cx),
                ),
            )
            .child(crate::ui::hint(t("onboarding_desktop_library_note"), cx))
            .into_any_element()
    }
}

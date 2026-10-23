//! Renders the daemon's own `FieldSpec` forms: source options, integrations, artwork providers, the downloader.

use std::collections::HashMap;
use std::rc::Rc;

use api::schema::{FieldKind, FieldSpec, FieldValue, Problem};
use gpui_kit::component::Disableable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::v_flex;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, IntoElement,
    ParentElement as _, Render, SharedString, Styled, Subscription, Window, div,
};

use crate::fields::{ComboChoice, FieldFrame, PathField, RadioGroup, SecretInput};
use crate::folders::{FolderList, pick_folders};
use crate::format::{t, text};
use crate::states::FieldProblem;
use crate::style;

/// One field was answered; the owner sends it to whichever `set_*_settings` call it belongs to.
#[derive(Debug, Clone)]
pub struct FieldChanged(pub FieldValue);

/// A form over daemon-supplied fields. The daemon owns which fields exist and what they mean.
pub struct SchemaForm {
    specs: Vec<FieldSpec>,
    values: HashMap<String, String>,
    inputs: HashMap<String, Entity<InputState>>,
    problems: Vec<Problem>,
    locked: Vec<String>,
    // Secret keys the daemon already holds a value for, since a secret never comes back to prefill.
    saved: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<FieldChanged> for SchemaForm {}

impl SchemaForm {
    pub fn new(specs: Vec<FieldSpec>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut form = Self {
            specs: Vec::new(),
            values: HashMap::new(),
            inputs: HashMap::new(),
            problems: Vec::new(),
            locked: Vec::new(),
            saved: Vec::new(),
            _subscriptions: Vec::new(),
        };
        form.set_specs(specs, window, cx);
        form
    }

    /// Replace the fields, keeping nothing: the daemon answered with the current values.
    pub fn set_specs(
        &mut self,
        specs: Vec<FieldSpec>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.values = specs
            .iter()
            .map(|spec| (spec.key.clone(), spec.value.clone().unwrap_or_default()))
            .collect();
        self.inputs.clear();
        self._subscriptions.clear();
        for spec in &specs {
            let free_text = matches!(&spec.kind, FieldKind::Choice { custom: true, .. });
            if !free_text
                && !matches!(
                    spec.kind,
                    FieldKind::Text | FieldKind::Secret | FieldKind::Url | FieldKind::Directory
                )
            {
                continue;
            }
            let key = spec.key.clone();
            let initial = spec.value.clone().unwrap_or_default();
            let placeholder = spec.placeholder.as_ref().map(text).unwrap_or_default();
            let input = cx.new(|cx| {
                let mut state = InputState::new(window, cx).placeholder(placeholder);
                // A secret is write-only, so its stored value never comes back to prefill.
                if !matches!(spec.kind, FieldKind::Secret) {
                    state = state.default_value(initial);
                }
                state
            });
            self._subscriptions.push(cx.subscribe(
                &input,
                move |this, state, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                        let value = state.read(cx).value().to_string();
                        this.answer(&key, value, cx);
                    }
                },
            ));
            self.inputs.insert(spec.key.clone(), input);
        }
        self.specs = specs;
        cx.notify();
    }

    /// Bring the form up to what the daemon last said: rebuilt only when its fields change, so a refetch never swaps an input out from under the caret.
    pub fn sync(
        &mut self,
        specs: Vec<FieldSpec>,
        locked: Vec<String>,
        secrets_held: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A secret never comes back, so the caller says whether any is stored.
        let saved = match secrets_held {
            true => specs
                .iter()
                .filter(|field| matches!(field.kind, FieldKind::Secret))
                .map(|field| field.key.clone())
                .collect(),
            false => Vec::new(),
        };
        if self.specs != specs {
            self.set_specs(specs, window, cx);
        }
        self.set_locked(locked, cx);
        self.set_saved(saved, cx);
    }

    pub fn set_problems(&mut self, problems: Vec<Problem>, cx: &mut Context<Self>) {
        self.problems = problems;
        cx.notify();
    }

    /// Secret keys the daemon already holds, so a blank box reads as "kept" rather than "unset".
    pub fn set_saved(&mut self, saved: Vec<String>, cx: &mut Context<Self>) {
        if self.saved == saved {
            return;
        }
        self.saved = saved;
        cx.notify();
    }

    /// Keys a managed settings file pins; their controls render disabled.
    pub fn set_locked(&mut self, locked: Vec<String>, cx: &mut Context<Self>) {
        self.locked = locked;
        cx.notify();
    }

    pub fn values(&self) -> Vec<FieldValue> {
        self.values
            .iter()
            .map(|(key, value)| FieldValue::new(key.clone(), value.clone()))
            .collect()
    }

    /// Take what is typed in every box as answered, as leaving it would; a button press does not blur it.
    pub fn commit_typed(&mut self, cx: &mut Context<Self>) {
        let typed: Vec<(String, String)> = self
            .inputs
            .iter()
            .map(|(key, input)| (key.clone(), input.read(cx).value().to_string()))
            .collect();
        self.values.extend(typed);
    }

    pub fn value(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// The daemon refused the answer to `key`: show what it last said it holds again.
    pub fn revert(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(spec) = self.specs.iter().find(|spec| spec.key == key) else {
            return;
        };
        let held = spec.value.clone().unwrap_or_default();
        if let Some(input) = self.inputs.get(key) {
            input.update(cx, |state, cx| state.set_value(held.clone(), window, cx));
        }
        self.values.insert(key.to_string(), held);
        cx.notify();
    }

    fn answer(&mut self, key: &str, value: String, cx: &mut Context<Self>) {
        if self.values.get(key).is_some_and(|held| held == &value) {
            return;
        }
        self.values.insert(key.to_string(), value.clone());
        cx.notify();
        cx.emit(FieldChanged(FieldValue::new(key.to_string(), value)));
    }

    fn is_locked(&self, spec: &FieldSpec) -> bool {
        spec.config_key
            .as_ref()
            .is_some_and(|key| self.locked.iter().any(|locked| locked == key))
    }

    /// `show_when` hides a field until another field holds the value it names.
    fn is_visible(&self, spec: &FieldSpec) -> bool {
        spec.show_when.as_ref().is_none_or(|condition| {
            self.values
                .get(&condition.key)
                .is_some_and(|value| value == &condition.value)
        })
    }

    fn problem_for(&self, key: &str) -> Option<SharedString> {
        self.problems
            .iter()
            .find(|problem| problem.field.as_deref() == Some(key))
            .map(|problem| text(&problem.label))
    }

    /// The control for one field, wired to answer through the form.
    fn control(
        &self,
        spec: &FieldSpec,
        value: &str,
        locked: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let key = spec.key.clone();
        let form = cx.entity().downgrade();
        let answer = {
            let (form, key) = (form.clone(), key.clone());
            Rc::new(move |value: String, cx: &mut App| {
                form.update(cx, |form, cx| form.answer(&key, value, cx))
                    .ok();
            })
        };
        Some(match &spec.kind {
            FieldKind::Note => return None,
            FieldKind::Toggle => Switch::new(SharedString::from(format!("toggle-{key}")))
                .checked(value == "true")
                .disabled(locked)
                .on_click(move |checked: &bool, _, cx| answer(checked.to_string(), cx))
                .into_any_element(),
            FieldKind::Directory => {
                let field = PathField::new(format!("browse-{key}"), value.to_string())
                    .locked(locked)
                    .on_change(move |path, cx| answer(path, cx));
                match self.inputs.get(&key) {
                    Some(input) => field.input(input),
                    None => field,
                }
                .into_any_element()
            }
            FieldKind::Directories => self.folders(&key, value, locked, cx),
            FieldKind::Radio { options } => options
                .iter()
                .fold(RadioGroup::new(key.clone(), value), |group, option| {
                    group.option(option.value.clone(), text(&option.label))
                })
                .locked(locked)
                .on_change(move |value, _, cx| answer(value.to_string(), cx))
                .into_any_element(),
            FieldKind::Choice { options, custom } => {
                let combo = options
                    .iter()
                    .fold(ComboChoice::new(key.clone(), value), |combo, option| {
                        combo.option(option.value.clone(), text(&option.label))
                    })
                    .locked(locked)
                    .on_pick({
                        let answer = answer.clone();
                        move |value, _, cx| answer(value.to_string(), cx)
                    });
                match self.inputs.get(&key).filter(|_| *custom) {
                    Some(input) => combo.custom(input.clone()).on_custom({
                        let input = input.clone();
                        move |window, cx| {
                            input.update(cx, |state, cx| state.set_value("", window, cx));
                            input.update(cx, |state, cx| state.focus(window, cx));
                            answer(String::new(), cx);
                        }
                    }),
                    None => combo,
                }
                .into_any_element()
            }
            FieldKind::Text | FieldKind::Url => match self.inputs.get(&key) {
                Some(input) => Input::new(input).disabled(locked).into_any_element(),
                None => div().into_any_element(),
            },
            FieldKind::Secret => match self.inputs.get(&key) {
                Some(input) => SecretInput::new(input)
                    .saved(self.saved.iter().any(|saved| saved == &key))
                    .locked(locked)
                    .into_any_element(),
                None => div().into_any_element(),
            },
        })
    }

    fn field(&self, spec: &FieldSpec, cx: &mut Context<Self>) -> impl IntoElement {
        let locked = self.is_locked(spec);
        let value = self.values.get(&spec.key).cloned().unwrap_or_default();
        let control = self.control(spec, &value, locked, cx);
        let cause = spec
            .config_key
            .as_deref()
            .filter(|_| locked)
            .and_then(|key| crate::settings::lock_cause(key, cx));
        FieldFrame::new(spec.key.clone(), text(&spec.label))
            .help(spec.help.as_ref().map(text))
            .locked(locked)
            .lock_cause(cause)
            .note(control.is_none())
            .problem(self.problem_for(&spec.key))
            .children(control)
    }

    /// The folders list, answering with the whole encoded list each time one is added or dropped.
    fn folders(&self, key: &str, value: &str, locked: bool, cx: &mut Context<Self>) -> AnyElement {
        let form = cx.entity().downgrade();
        FolderList::new(
            SharedString::from(format!("folders-{key}")),
            api::schema::decode_directories(value),
        )
        .locked(locked)
        .empty(t("no_music_folders"))
        .on_remove({
            let (form, key) = (form.clone(), key.to_string());
            move |path, _, cx| {
                form.update(cx, |form, cx| {
                    let held = form.values.get(&key).cloned().unwrap_or_default();
                    let mut paths = api::schema::decode_directories(&held);
                    paths.retain(|kept| kept != path);
                    form.answer(&key, api::schema::encode_directories(&paths), cx);
                })
                .ok();
            }
        })
        .on_add({
            let key = key.to_string();
            move |_, _, cx| {
                let (form, key) = (form.clone(), key.clone());
                pick_folders(true, cx, move |picked, cx| {
                    form.update(cx, |form, cx| {
                        let held = form.values.get(&key).cloned().unwrap_or_default();
                        let mut paths = api::schema::decode_directories(&held);
                        for path in picked {
                            if !paths.contains(&path) {
                                paths.push(path);
                            }
                        }
                        form.answer(&key, api::schema::encode_directories(&paths), cx);
                    })
                    .ok();
                });
            }
        })
        .into_any_element()
    }
}

impl Render for SchemaForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut section: Option<SharedString> = None;
        let mut rows = Vec::new();
        for spec in self.specs.clone() {
            if !self.is_visible(&spec) {
                continue;
            }
            let heading = spec.section.as_ref().map(text);
            if heading.is_some() && heading != section {
                section = heading.clone();
                // `uppercase` is a CSS transform GPUI has no equivalent for.
                let title = heading.clone().unwrap_or_default().to_uppercase();
                rows.push(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_1()
                        .text_xs()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(style::white(style::TEXT_MUTED, cx))
                        .child(SharedString::from(title))
                        .into_any_element(),
                );
            }
            rows.push(self.field(&spec, cx).into_any_element());
        }
        let form_problems: Vec<SharedString> = self
            .problems
            .iter()
            .filter(|problem| problem.field.is_none())
            .map(|problem| text(&problem.label))
            .collect();
        v_flex().w_full().children(rows).children(
            form_problems
                .into_iter()
                .map(|problem| div().px_5().py_1().child(FieldProblem::new(problem))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::schema::{Text, encode_directories};

    fn folders(held: &[&str]) -> FieldSpec {
        let held: Vec<String> = held.iter().map(|path| path.to_string()).collect();
        FieldSpec {
            key: "directories".into(),
            label: Text::literal("Folders"),
            help: None,
            placeholder: None,
            section: None,
            kind: FieldKind::Directories,
            required: false,
            value: Some(encode_directories(&held)),
            config_key: Some("local_sources".into()),
            show_when: None,
        }
    }

    #[gpui_kit::test]
    fn a_refused_answer_shows_what_the_daemon_holds_again(cx: &mut gpui_kit::TestAppContext) {
        let spec = folders(&["/music", "/more"]);
        let held = spec.value.clone();
        let view = cx.add_window(|_, _| gpui_kit::Empty);
        cx.update_window(view.into(), |_, window, cx| {
            let form = cx.new(|cx| SchemaForm::new(vec![spec], window, cx));
            form.update(cx, |form, cx| {
                form.answer("directories", encode_directories(&["/more".into()]), cx);
                assert_ne!(form.value("directories"), held.as_deref());
                form.revert("directories", window, cx);
                assert_eq!(form.value("directories"), held.as_deref());
            });
        })
        .unwrap();
    }
}

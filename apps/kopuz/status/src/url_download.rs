//! Fetching a URL to a file: the tool, the formats and every option are the daemon's.

use api::ApiEvent;
use api::schema::{ChoiceOption, FieldValue};
use components::ErrorBanner;
use components::dialogs::FormDialog;
use components::format::{t, text};
use components::{Chip, ChipSize, FieldChanged, SchemaForm, icons, style};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Task, Window, div, px,
};
use store::{Link, Notices, Query};

const OPTIONS_HEIGHT: Pixels = px(240.);

/// Wire the box the popover's chip and `downloads.add_url` both open.
pub fn register(cx: &mut App) {
    store::URL_DOWNLOAD.register(|(), window, cx| open(window, cx), cx);
}

pub(crate) fn open(window: &mut Window, cx: &mut App) {
    let dialog = cx.new(|cx| UrlDownload::new(window, cx));
    let field = dialog.read(cx).url.clone();
    let start = dialog.clone();
    FormDialog::new(t("downloader_url_title"))
        .on_submit(move |window, cx| start.update(cx, |this, cx| this.start(window, cx)))
        .open(dialog, window, cx);
    field.update(cx, |state, cx| state.focus(window, cx));
}

/// A URL, a format, the daemon's own options, and whatever it refused with.
pub(crate) struct UrlDownload {
    url: Entity<InputState>,
    formats: Entity<Query<Vec<ChoiceOption>>>,
    format: Option<String>,
    options: Entity<SchemaForm>,
    show_options: bool,
    starting: bool,
    failure: Option<SharedString>,
    _start: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl UrlDownload {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let url =
            cx.new(|cx| InputState::new(window, cx).placeholder(t("downloader_url_placeholder")));
        let formats =
            cx.new(|cx| Query::new(&[], |api| async move { api.download_formats().await }, cx));
        let options = cx.new(|cx| SchemaForm::new(Vec::new(), window, cx));
        let connection = Link::connection(cx);

        let subscriptions = vec![
            cx.subscribe(&url, |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this.clear_failure(cx)
                }
            }),
            cx.observe(&formats, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &options,
                window,
                |this, _, changed: &FieldChanged, window, cx| {
                    this.save_setting(changed.0.clone(), window, cx)
                },
            ),
            cx.subscribe_in(
                &connection,
                window,
                |this, _, event: &ApiEvent, window, cx| {
                    if matches!(event, ApiEvent::Resync) {
                        this.load_settings(window, cx);
                    }
                },
            ),
        ];

        let mut dialog = Self {
            url,
            formats,
            format: None,
            options,
            show_options: false,
            starting: false,
            failure: None,
            _start: None,
            _subscriptions: subscriptions,
        };
        dialog.load_settings(window, cx);
        dialog
    }

    /// The picked format, or the first the daemon offers: the box never names one itself.
    fn format(&self, cx: &Context<Self>) -> Option<String> {
        let offered = self.formats.read(cx).value();
        self.format
            .clone()
            .filter(|picked| {
                offered.is_some_and(|options| options.iter().any(|option| &option.value == picked))
            })
            .or_else(|| {
                offered
                    .and_then(|options| options.first())
                    .map(|option| option.value.clone())
            })
    }

    fn clear_failure(&mut self, cx: &mut Context<Self>) {
        if self.failure.take().is_some() {
            cx.notify();
        }
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.url.read(cx).value().trim().to_string();
        if url.is_empty() || self.starting {
            return;
        }
        let format = self.format(cx).unwrap_or_default();
        self.starting = true;
        self.failure = None;
        cx.notify();
        let call =
            Link::global(cx).call(move |api| async move { api.download_url(url, format).await });
        self._start = Some(cx.spawn_in(window, async move |this, cx| {
            // Whether the tool is there and the folder writable is the daemon's to check.
            let result = call.await;
            this.update_in(cx, |this, window, cx| {
                this.starting = false;
                match result {
                    Ok(_) => window.close_dialog(cx),
                    // A refusal keeps the typed URL on screen with the reason beside it.
                    Err(error) => {
                        tracing::warn!(%error, "the download was refused");
                        this.failure = Some(store::human(&error));
                        cx.notify();
                    }
                }
            })
            .ok();
        }));
    }

    fn load_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.downloader_settings().await });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(specs) = call.await else { return };
            this.update_in(cx, |this, window, cx| {
                this.options
                    .update(cx, |form, cx| form.set_specs(specs, window, cx));
            })
            .ok();
        })
        .detach();
    }

    fn save_setting(&mut self, value: FieldValue, window: &mut Window, cx: &mut Context<Self>) {
        let key = value.key.clone();
        let call = Link::global(cx)
            .call(move |api| async move { api.set_downloader_settings(vec![value]).await });
        cx.spawn_in(window, async move |this, cx| match call.await {
            Ok(specs) => {
                this.update_in(cx, |this, window, cx| {
                    this.options
                        .update(cx, |form, cx| form.set_specs(specs, window, cx));
                })
                .ok();
            }
            Err(error) => {
                tracing::warn!(%error, "saving a downloader setting failed");
                this.update_in(cx, |this, window, cx| {
                    this.options
                        .update(cx, |form, cx| form.revert(&key, window, cx));
                    Notices::failed(&error, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// One tab stop per chip: the arrows here would seek, so Tab and Enter are the walk.
    fn chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let picked = self.format(cx);
        let formats = self.formats.read(cx).value().cloned().unwrap_or_default();
        let chips: Vec<Chip> = formats
            .iter()
            .map(|option| {
                let pick = option.value.clone();
                Chip::new(SharedString::from(format!("url-format-{}", option.value)))
                    .label(text(&option.label))
                    .size(ChipSize::Medium)
                    .selected(picked.as_deref() == Some(option.value.as_str()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.format = Some(pick.clone());
                        cx.notify();
                    }))
            })
            .collect();
        h_flex().gap_1p5().flex_wrap().children(chips)
    }
}

impl Render for UrlDownload {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.url.read(cx).value().trim().is_empty();
        let chevron = match self.show_options {
            true => icons::CHEVRON_UP,
            false => icons::CHEVRON_DOWN,
        };
        v_flex()
            .w_full()
            .gap_3()
            .child(
                Input::new(&self.url)
                    .bg(style::white(style::HOVER, cx))
                    .border_color(style::white(style::BORDER, cx))
                    .rounded(style::radius_lg())
                    .px_3()
                    .py_2()
                    .text_sm(),
            )
            .child(self.chips(cx))
            .when_some(self.failure.clone(), |body, failure| {
                body.child(ErrorBanner::new(failure).icon())
            })
            .child(
                h_flex().child(
                    Button::new("url-download-options")
                        .custom(style::quiet_button(style::slate_400(cx), cx))
                        .xsmall()
                        .icon(icons::icon(chevron))
                        .label(t("downloader_options"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_options = !this.show_options;
                            cx.notify();
                        })),
                ),
            )
            .when(self.show_options, |body| {
                body.child(
                    div()
                        .id("url-download-options-panel")
                        .max_h(OPTIONS_HEIGHT)
                        .overflow_y_scroll()
                        .px_3()
                        .py_2()
                        .rounded(style::radius_lg())
                        .border_1()
                        .border_color(style::white(style::BORDER, cx))
                        .bg(style::white(style::RAISED, cx))
                        .child(self.options.clone()),
                )
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("url-download-cancel")
                            .ghost()
                            .label(t("cancel"))
                            .disabled(self.starting)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("url-download-start")
                            .primary()
                            .icon(icons::icon(icons::DOWNLOAD))
                            .label(t("downloader_download"))
                            .loading(self.starting)
                            .disabled(self.starting || empty)
                            .on_click(cx.listener(|this, _, window, cx| this.start(window, cx))),
                    ),
            )
    }
}

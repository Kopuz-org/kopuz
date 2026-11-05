//! Fetching a URL to a file: the tool, the formats and every option are the daemon's.

use std::collections::HashSet;

use api::schema::{ChoiceOption, FieldValue};
use api::{ApiEvent, DownloadCandidate};
use components::dialogs::FormDialog;
use components::format::{duration, t, t_with, text};
use components::{
    Chip, ChipSize, Cover, ErrorBanner, FieldChanged, IconButton, Loading, SchemaForm, icons, style,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _,
    Styled, Subscription, Task, Window, div, px,
};
use store::{Link, Notices, Query};

const OPTIONS_HEIGHT: Pixels = px(240.);
const RESULTS_HEIGHT: Pixels = px(288.);
const RESULT_COVER: Pixels = px(40.);

/// What the last search asked for, and what came back once it has.
struct Search {
    query: String,
    found: Option<Result<Vec<DownloadCandidate>, SharedString>>,
}

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
    search: Option<Search>,
    // The candidates already handed to the downloader, so a second press cannot fetch one twice.
    fetching: HashSet<String>,
    fetched: HashSet<String>,
    _start: Option<Task<()>>,
    _search: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl UrlDownload {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let url =
            cx.new(|cx| InputState::new(window, cx).placeholder(t("downloader_url_placeholder")));
        let formats =
            cx.new(|cx| Query::new(&[], |api| async move { api.download_formats().await }, cx));
        let options = cx.new(|_| SchemaForm::empty());
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
            cx.subscribe(&connection, |this, _, event: &ApiEvent, cx| {
                if matches!(event, ApiEvent::Resync) {
                    this.load_settings(cx);
                }
            }),
        ];

        let mut dialog = Self {
            url,
            formats,
            format: None,
            options,
            show_options: false,
            starting: false,
            failure: None,
            search: None,
            fetching: HashSet::new(),
            fetched: HashSet::new(),
            _start: None,
            _search: None,
            _subscriptions: subscriptions,
        };
        dialog.load_settings(cx);
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

    /// A name finds songs to fetch; a pasted link lists the tracks it stands for.
    fn search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.url.read(cx).value().trim().to_string();
        if query.is_empty() {
            return;
        }
        self.failure = None;
        self.search = Some(Search {
            query: query.clone(),
            found: None,
        });
        cx.notify();
        let call =
            Link::global(cx).call(move |api| async move { api.search_downloads(query).await });
        self._search = Some(cx.spawn_in(window, async move |this, cx| {
            let found = call.await.map_err(|error| store::human(&error));
            this.update(cx, |this, cx| {
                if let Some(search) = this.search.as_mut() {
                    search.found = Some(found);
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// One result's own download; the dialog stays open so the next can follow.
    fn fetch(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.fetching.insert(url.clone()) {
            return;
        }
        self.failure = None;
        cx.notify();
        let format = self.format(cx).unwrap_or_default();
        let call = {
            let url = url.clone();
            Link::global(cx).call(move |api| async move { api.download_url(url, format).await })
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                this.fetching.remove(&url);
                match result {
                    Ok(_) => {
                        this.fetched.insert(url);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the download was refused");
                        this.failure = Some(store::human(&error));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn results(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let search = self.search.as_ref()?;
        let element = match &search.found {
            None => Loading::new().compact().into_any_element(),
            Some(Err(error)) => ErrorBanner::new(error.clone()).icon().into_any_element(),
            Some(Ok(found)) if found.is_empty() => div()
                .py_4()
                .text_sm()
                .text_center()
                .text_color(style::slate_500(cx))
                .child(t_with(
                    "no_results_found",
                    &[("query", search.query.clone().into())],
                ))
                .into_any_element(),
            Some(Ok(found)) => {
                let rows: Vec<_> = found
                    .iter()
                    .enumerate()
                    .map(|(position, candidate)| self.result(position, candidate, cx))
                    .collect();
                div()
                    .id("url-download-results")
                    .max_h(RESULTS_HEIGHT)
                    .overflow_y_scroll()
                    .rounded(style::radius_lg())
                    .border_1()
                    .border_color(style::white(style::BORDER, cx))
                    .child(v_flex().children(rows))
                    .into_any_element()
            }
        };
        Some(element)
    }

    fn result(
        &self,
        position: usize,
        candidate: &DownloadCandidate,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let byline = [candidate.artist.as_str(), candidate.album.as_str()]
            .into_iter()
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let fetching = self.fetching.contains(&candidate.url);
        let fetched = self.fetched.contains(&candidate.url);
        let url = candidate.url.clone();
        let icon = match fetched {
            true => icons::CHECK,
            false => icons::DOWNLOAD,
        };
        h_flex()
            .id(SharedString::from(format!(
                "url-result-{position}-{}",
                candidate.url
            )))
            .gap_3()
            .px_3()
            .py_2()
            .items_center()
            .hover(|row| row.bg(style::white(style::RAISED, cx)))
            // The covers are remote URLs, and this app reads pictures only through the daemon.
            .child(
                Cover::new(None)
                    .size(RESULT_COVER)
                    .rounded(style::cover_radius())
                    .placeholder(icons::MUSIC),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(candidate.title.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .text_color(style::slate_500(cx))
                            .child(byline),
                    ),
            )
            .when(candidate.duration_secs > 0, |row| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(style::slate_500(cx))
                        .child(duration(candidate.duration_secs * 1000)),
                )
            })
            .child(
                IconButton::new(SharedString::from(format!("url-fetch-{position}")), icon)
                    .tooltip(t("downloader_download"))
                    .loading(fetching)
                    .disabled(fetching || fetched)
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.fetch(url.clone(), window, cx)),
                    ),
            )
            .into_any_element()
    }

    fn load_settings(&mut self, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.downloader_settings().await });
        cx.spawn(async move |this, cx| {
            let Ok(specs) = call.await else { return };
            this.update(cx, |this, cx| {
                this.options
                    .update(cx, |form, cx| form.set_specs(specs, cx));
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
                this.update(cx, |this, cx| {
                    this.options
                        .update(cx, |form, cx| form.set_specs(specs, cx));
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
        let searching = self
            .search
            .as_ref()
            .is_some_and(|search| search.found.is_none());
        let results = self.results(cx);
        let chevron = match self.show_options {
            true => icons::CHEVRON_UP,
            false => icons::CHEVRON_DOWN,
        };
        v_flex()
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Input::new(&self.url)
                            .flex_1()
                            .bg(style::white(style::HOVER, cx))
                            .border_color(style::white(style::BORDER, cx))
                            .rounded(style::radius_lg())
                            .px_3()
                            .py_2()
                            .text_sm(),
                    )
                    .child(
                        IconButton::new("url-download-search", icons::SEARCH)
                            .tooltip(t("search"))
                            .loading(searching)
                            .disabled(empty || searching)
                            .on_click(cx.listener(|this, _, window, cx| this.search(window, cx))),
                    ),
            )
            .child(self.chips(cx))
            .when_some(self.failure.clone(), |body, failure| {
                body.child(ErrorBanner::new(failure).icon())
            })
            .children(results)
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

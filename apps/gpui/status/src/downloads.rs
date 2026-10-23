//! The download queue, and the history of everything fetched from a URL.

use std::ops::Range;

use api::schema::ChoiceOption;
use api::{DownloadHistoryEntry, DownloadState, JobKind};
use components::format::{t, t_with, text};
use components::{Chip, ChipTone, JobStop, ProgressBar, StatusRow, icons, style};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, IntoElement,
    ParentElement as _, Pixels, Render, SharedString, Styled, Subscription,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use store::{DownloadItem, DownloadsModel, ItemState, JobEvent, JobsModel, Link, Notices, Query};

use crate::{DONE, FAILED, header};
use components::EmptyState;

/// The same one-line list row the playlists panel draws, so a skin moves both together.
fn row_height() -> Pixels {
    style::list_row()
}
/// `LIST_HEIGHT` as a number, so a short queue can size its list to itself.
const LIST: f32 = 260.;
/// The history shares the popover with the queue, so it gets the shorter half.
const HISTORY: f32 = 170.;

/// The popover asking to stand down, because what it opened wants the screen.
pub(crate) struct Dismiss;

/// The downloads popover's body.
pub struct DownloadsList {
    scroll: UniformListScrollHandle,
    past_scroll: UniformListScrollHandle,
    history: Entity<Query<Vec<DownloadHistoryEntry>>>,
    formats: Entity<Query<Vec<ChoiceOption>>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Dismiss> for DownloadsList {}

impl DownloadsList {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let downloads = DownloadsModel::global(cx);
        let jobs = JobsModel::global(cx);
        let history =
            cx.new(|cx| Query::new(&[], |api| async move { api.downloader_history().await }, cx));
        let formats =
            cx.new(|cx| Query::new(&[], |api| async move { api.download_formats().await }, cx));
        Self {
            scroll: UniformListScrollHandle::new(),
            past_scroll: UniformListScrollHandle::new(),
            history: history.clone(),
            formats,
            _subscriptions: vec![
                cx.observe(&downloads, |_, _, cx| cx.notify()),
                cx.observe(&jobs, |_, _, cx| cx.notify()),
                cx.observe(&history, |_, _, cx| cx.notify()),
                // The history gains a row the moment a URL download stops running.
                cx.subscribe(&jobs, |this, _, event: &JobEvent, cx| {
                    if event.kind == JobKind::UrlDownload {
                        this.history.update(cx, |history, cx| history.refresh(cx));
                    }
                }),
            ],
        }
    }

    /// The popover was opened: what the daemon fetched while it was shut is worth re-reading.
    pub(crate) fn opened(&mut self, cx: &mut Context<Self>) {
        self.history.update(cx, |history, cx| history.refresh(cx));
    }

    fn clear_history(&mut self, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.clear_downloader_history().await });
        cx.spawn(async move |this, cx| match call.await {
            Ok(()) => {
                this.update(cx, |this, cx| {
                    this.history.update(cx, |history, cx| history.refresh(cx));
                })
                .ok();
            }
            Err(error) => {
                tracing::warn!(%error, "clearing the download history failed");
                this.update(cx, |_, cx| Notices::failed(&error, cx)).ok();
            }
        })
        .detach();
    }

    /// The chip that opens the URL box; a tab stop, because a popover is reachable without a mouse.
    fn add_url(&self, cx: &mut Context<Self>) -> AnyElement {
        Chip::new("downloads-add-url")
            .label(t("downloader_add_url"))
            .tone(ChipTone::Ink(cx.theme().primary))
            .on_click(cx.listener(|_, _, window, cx| {
                cx.emit(Dismiss);
                store::URL_DOWNLOAD.open((), window, cx);
            }))
            .into_any_element()
    }

    fn past_row(
        &self,
        entry: &DownloadHistoryEntry,
        formats: &[ChoiceOption],
        cx: &App,
    ) -> AnyElement {
        let failed = entry.state == DownloadState::Failed;
        let status: SharedString = match (failed, entry.error.clone()) {
            (true, Some(error)) => error.into(),
            (true, None) => t("downloader_status_failed"),
            (false, _) => t("downloader_status_completed"),
        };
        let format = formats
            .iter()
            .find(|option| option.value == entry.format)
            .map(|option| text(&option.label))
            .unwrap_or_default();
        let title: SharedString = match entry.title == entry.url {
            true => shorten(&entry.url),
            false => entry.title.clone().into(),
        };
        let (glyph, ink) = match failed {
            true => (FAILED, cx.theme().danger),
            false => (DONE, cx.theme().success),
        };
        let status_ink = match failed {
            true => cx.theme().danger,
            false => style::slate_500(cx),
        };
        StatusRow::new(format!("past-{}", entry.url))
            .height(row_height())
            .glyph(glyph, ink)
            .title(title)
            .aside(format)
            .detail(status, Some(status_ink))
            .into_any_element()
    }

    fn row(&self, item: &DownloadItem, cx: &App) -> AnyElement {
        let (glyph, ink) = match item.state {
            ItemState::Downloading => (icons::DOWNLOAD, cx.theme().primary_hover),
            ItemState::Queued => (icons::CLOCK, style::slate_500(cx)),
            ItemState::Failed => (FAILED, cx.theme().danger),
            ItemState::Done => (DONE, cx.theme().success),
        };
        let title_ink = match item.state {
            ItemState::Failed => cx.theme().danger,
            ItemState::Queued => style::slate_500(cx),
            _ => cx.theme().foreground,
        };
        StatusRow::new(format!("download-{}", item.key))
            .height(row_height())
            .glyph(glyph, ink)
            .title(item.title.clone())
            .title_ink(title_ink)
            .when(!item.artist.is_empty(), |row| {
                row.detail(item.artist.clone(), Some(style::slate_500(cx)))
            })
            .into_any_element()
    }
}

impl Render for DownloadsList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let downloads = DownloadsModel::global(cx);
        let (count, failed, batch) = {
            let downloads = downloads.read(cx);
            let queue = downloads.queue();
            let failed = queue
                .iter()
                .filter(|item| item.state == ItemState::Failed)
                .count();
            (queue.len(), failed, downloads.batch())
        };
        let running = JobsModel::global(cx)
            .read(cx)
            .progress(JobKind::Download)
            .cloned();

        let action = match (running.as_ref(), count > 0) {
            (Some(job), _) => Some(JobStop::new(job.id.clone()).into_any_element()),
            (None, true) => Some(
                Chip::new("clear-downloads")
                    .label(t("clear"))
                    .tone(ChipTone::Ink(style::slate_400(cx)))
                    .on_click(|_, _, cx| {
                        DownloadsModel::global(cx)
                            .update(cx, |downloads, cx| downloads.clear_queue(cx));
                    })
                    .into_any_element(),
            ),
            _ => None,
        };
        let past = self.history.clone();
        let formats = self.formats.clone();
        let fetched = past.read(cx).value().map_or(0, Vec::len);
        let heading = h_flex()
            .gap_1p5()
            .items_center()
            .children(action)
            .child(self.add_url(cx))
            .into_any_element();

        v_flex()
            .w_full()
            .child(header(t("downloads"), Some(heading), cx))
            .when_some(running.as_ref(), |body, job| {
                body.child(ProgressBar::job(job))
            })
            .when_some(batch, |body, batch| {
                body.child(
                    div()
                        .w_full()
                        .px_3()
                        .py_1p5()
                        .text_size(style::text_10())
                        .text_color(style::white(style::TEXT_SECONDARY, cx))
                        .child(t_with(
                            "downloads_summary",
                            &[
                                ("done", batch.downloaded.into()),
                                ("failed", batch.failed.into()),
                            ],
                        )),
                )
            })
            .when(count == 0, |body| {
                body.child(EmptyState::new(t("downloads_idle")).compact())
            })
            .when(count > 0, |body| {
                body.child(
                    uniform_list(
                        "download-rows",
                        count,
                        cx.processor(move |this, range: Range<usize>, _, cx| {
                            let queue = downloads.read(cx).queue();
                            range
                                .filter_map(|index| queue.get(index).map(|item| this.row(item, cx)))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(&self.scroll)
                    .h(px(LIST.min(row_height().to_f64() as f32 * count as f32))),
                )
            })
            .when(failed > 0 && running.is_none(), |body| {
                body.child(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .px_3()
                        .py_2()
                        .border_t_1()
                        .border_color(style::white(style::BORDER_SOFT, cx))
                        .child(
                            Chip::new("retry-downloads")
                                .label(t("retry_failed"))
                                .tone(ChipTone::Ink(cx.theme().primary))
                                .on_click(|_, _, cx| {
                                    DownloadsModel::global(cx)
                                        .update(cx, |downloads, cx| downloads.retry_failed(cx));
                                }),
                        ),
                )
            })
            .when(fetched > 0, |body| {
                body.child(
                    div()
                        .w_full()
                        .border_t_1()
                        .border_color(style::white(style::BORDER_SOFT, cx))
                        .child(header(
                            t("downloader_history"),
                            Some(
                                Chip::new("clear-url-history")
                                    .label(t("clear"))
                                    .tone(ChipTone::Ink(style::slate_400(cx)))
                                    .on_click(cx.listener(|this, _, _, cx| this.clear_history(cx)))
                                    .into_any_element(),
                            ),
                            cx,
                        )),
                )
                .child(
                    uniform_list(
                        "url-download-rows",
                        fetched,
                        cx.processor(move |this, range: Range<usize>, _, cx| {
                            let past = past.read(cx).value().map(Vec::as_slice).unwrap_or_default();
                            let formats = formats
                                .read(cx)
                                .value()
                                .map(Vec::as_slice)
                                .unwrap_or_default();
                            range
                                .filter_map(|index| {
                                    past.get(index)
                                        .map(|entry| this.past_row(entry, formats, cx))
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(&self.past_scroll)
                    .h(px(
                        HISTORY.min(row_height().to_f64() as f32 * fetched as f32)
                    )),
                )
            })
    }
}

/// A URL is what a row shows until the downloader names the file it wrote.
fn shorten(url: &str) -> SharedString {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .chars()
        .take(48)
        .collect::<String>()
        .into()
}

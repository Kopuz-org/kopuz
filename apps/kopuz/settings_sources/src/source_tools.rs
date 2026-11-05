//! What a source card offers past its options: the account it plays as, a connection test, cloud storage.

use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use api::{CheckOutcome, CheckStep, ConnectionCheck, JobKind, SourceAccount, StorageQuota, Table};
use components::format::{t, t_with};
use components::settings::Choice;
use components::{icons, style};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Div, Entity, IntoElement, ParentElement as _,
    PathPromptOptions, Render, SharedString, Styled, Subscription, Window, div,
};
use link::Link;
use store::{JobsModel, Notices, Query, SourcesModel};

use crate::ui;

/// `text-white/90` on a test step, `text-white/50` on what the source answered.
const STEP: f32 = 0.90;
const DETAIL: f32 = 0.50;

/// The `accept` list the Dioxus upload input offered.
const AUDIO: &[&str] = &["mp3", "m4a", "flac", "wav", "ogg", "opus", "wma", "aiff"];

/// The rule over each block, the way the card already separates its folders and options.
fn block(cx: &App) -> Div {
    v_flex()
        .w_full()
        .gap_2()
        .pt_2()
        .border_t_1()
        .border_color(cx.theme().border)
}

fn failure(message: SharedString, cx: &App) -> Div {
    div().text_xs().text_color(cx.theme().danger).child(message)
}

pub struct AccountPicker {
    source_id: String,
    accounts: Entity<Query<Vec<SourceAccount>>>,
    /// The account just picked, held until the daemon has answered for it.
    chosen: Option<Option<String>>,
    _subscription: Subscription,
}

impl AccountPicker {
    pub fn new(source_id: String, cx: &mut Context<Self>) -> Self {
        let id = source_id.clone();
        let accounts = cx.new(|cx| {
            Query::new(
                &[Table::Servers],
                move |api| {
                    let id = id.clone();
                    async move { api.accounts(id).await }
                },
                cx,
            )
        });
        let subscription = cx.observe(&accounts, |_, _, cx| cx.notify());
        Self {
            source_id,
            accounts,
            chosen: None,
            _subscription: subscription,
        }
    }

    fn switch(&mut self, account: Option<String>, cx: &mut Context<Self>) {
        self.chosen = Some(account.clone());
        cx.notify();
        let id = self.source_id.clone();
        let call =
            Link::global(cx).call(move |api| async move { api.switch_account(id, account).await });
        cx.spawn(async move |this, cx| match call.await {
            Ok(_) => {
                cx.update(|cx| {
                    SourcesModel::global(cx).update(cx, |sources, cx| sources.refetch(cx));
                });
            }
            Err(error) => {
                tracing::warn!(%error, "switching the source account failed");
                this.update(cx, |this, cx| {
                    this.chosen = None;
                    Notices::failed(&error, cx);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }
}

impl Render for AccountPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.accounts.read(cx);
        if let Some(error) = query.error() {
            let message = t_with(
                "source_accounts_failed",
                &[("error", store::human(error).to_string().into())],
            );
            return block(cx).child(failure(message, cx));
        }
        // One account is the signed-in one, with nothing to switch to.
        let Some(accounts) = query.value().filter(|accounts| accounts.len() > 1) else {
            return div();
        };
        let active = self.chosen.clone().unwrap_or_else(|| {
            accounts
                .iter()
                .find(|account| account.active)
                .and_then(|account| account.id.clone())
        });
        let options = accounts
            .iter()
            .map(|account| {
                let label = match &account.handle {
                    Some(handle) => format!("{} ({handle})", account.name),
                    None => account.name.clone(),
                };
                (account.id.clone().unwrap_or_default(), label.into())
            })
            .collect();
        let this = cx.entity().downgrade();
        let current = active.clone();
        block(cx)
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .justify_between()
                    .child(ui::label(t("source_account")))
                    .child(
                        Choice::new(
                            SharedString::from(format!("account-{}", self.source_id)),
                            active.unwrap_or_default(),
                            options,
                        )
                        .on_change(move |picked: String, cx| {
                            let account = (!picked.is_empty()).then_some(picked);
                            if account == current {
                                return;
                            }
                            this.update(cx, |this, cx| this.switch(account, cx)).ok();
                        }),
                    ),
            )
            .child(ui::hint(t("source_account_help"), cx))
    }
}

/// Runs the active source's connection test and lists each step, so a bad day says which part broke.
pub struct ConnectionTest {
    source_id: String,
    running: bool,
    report: Option<Result<Vec<ConnectionCheck>, SharedString>>,
}

impl ConnectionTest {
    pub fn new(source_id: String) -> Self {
        Self {
            source_id,
            running: false,
            report: None,
        }
    }

    fn run(&mut self, cx: &mut Context<Self>) {
        self.running = true;
        self.report = None;
        cx.notify();
        let id = self.source_id.clone();
        let call =
            Link::global(cx).call(move |api| async move { api.test_source_connection(id).await });
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|error| store::human(&error));
            this.update(cx, |this, cx| {
                this.running = false;
                this.report = Some(result);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn check(check: &ConnectionCheck, cx: &App) -> impl IntoElement {
        let (glyph, color) = match check.outcome {
            CheckOutcome::Passed => (icons::CHECK, cx.theme().success),
            CheckOutcome::Failed => (icons::CLOSE, cx.theme().danger),
            CheckOutcome::Skipped => (icons::MORE, style::white(DETAIL, cx)),
        };
        h_flex()
            .items_start()
            .gap_2()
            .text_xs()
            .child(icons::icon(glyph).text_color(color).flex_shrink_0())
            .child(
                v_flex()
                    .min_w_0()
                    .child(
                        div()
                            .text_color(style::white(STEP, cx))
                            .child(step_label(check.step)),
                    )
                    .child(
                        div()
                            .text_color(style::white(DETAIL, cx))
                            .child(check.detail.clone()),
                    ),
            )
    }
}

impl Render for ConnectionTest {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = &self.source_id;
        let checks = match &self.report {
            Some(Ok(checks)) => Some(checks.clone()),
            _ => None,
        };
        block(cx)
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("test-{id}")))
                            .custom(style::soft_button(cx.theme().foreground, cx))
                            .xsmall()
                            .label(match self.running {
                                true => t("connection_testing"),
                                false => t("connection_test"),
                            })
                            .disabled(self.running)
                            .on_click(cx.listener(|this, _, _, cx| this.run(cx))),
                    )
                    .when_some(checks.clone(), |row, checks| {
                        row.child(
                            Button::new(SharedString::from(format!("test-copy-{id}")))
                                .ghost()
                                .xsmall()
                                .label(t("connection_copy_report"))
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(report_text(
                                        &checks,
                                    )));
                                    Notices::info(t("connection_report_copied"), cx);
                                }),
                        )
                    }),
            )
            .when_some(checks, |column, checks| {
                column.children(checks.iter().map(|check| Self::check(check, cx)))
            })
            .when_some(
                self.report
                    .as_ref()
                    .and_then(|report| report.as_ref().err()),
                |column, error| {
                    column.child(failure(
                        t_with(
                            "connection_test_failed",
                            &[("error", error.to_string().into())],
                        ),
                        cx,
                    ))
                },
            )
    }
}

fn step_label(step: CheckStep) -> SharedString {
    t(match step {
        CheckStep::Reach => "connection_step_reach",
        CheckStep::Session => "connection_step_session",
        CheckStep::SignedInPlayback => "connection_step_signed_in_playback",
        CheckStep::AnonymousPlayback => "connection_step_anonymous_playback",
        CheckStep::PlaybackToken => "connection_step_playback_token",
    })
}

/// English and in step order whatever the UI language, since it is read by whoever answers the bug report.
fn report_text(checks: &[ConnectionCheck]) -> String {
    let mut text = utils::build_info::summary();
    for check in checks {
        text.push_str(&format!(
            "\n{:?}: {:?}: {}",
            check.step, check.outcome, check.detail
        ));
    }
    text
}

/// The source's cloud storage: how full it is, and an upload when the source takes one.
pub struct CloudStorage {
    source_id: String,
    uploads: bool,
    quota: Entity<Query<StorageQuota>>,
    uploading: Option<SharedString>,
    _subscription: Subscription,
}

impl CloudStorage {
    pub fn new(source_id: String, cx: &mut Context<Self>) -> Self {
        let id = source_id.clone();
        let quota = cx.new(|cx| {
            Query::new(
                &[Table::Servers],
                move |api| {
                    let id = id.clone();
                    async move { api.storage_quota(id).await }
                },
                cx,
            )
        });
        let subscription = cx.observe(&quota, |_, _, cx| cx.notify());
        Self {
            source_id,
            uploads: false,
            quota,
            uploading: None,
            _subscription: subscription,
        }
    }

    pub fn set_uploads(&mut self, uploads: bool, cx: &mut Context<Self>) {
        if self.uploads != uploads {
            self.uploads = uploads;
            cx.notify();
        }
    }

    fn pick(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let paths: Vec<PathBuf> = paths.into_iter().filter(|path| is_audio(path)).collect();
            if paths.is_empty() {
                return;
            }
            this.update(cx, |this, cx| this.upload(paths, cx)).ok();
        })
        .detach();
    }

    /// One file at a time, stopping at the first that fails, as the Dioxus page did.
    fn upload(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let id = self.source_id.clone();
        let link = Link::global(cx).clone();
        cx.spawn(async move |this, cx| {
            let mut uploaded: i64 = 0;
            for path in paths {
                let filename = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let shown = SharedString::from(filename.clone());
                let started = this.update(cx, |this, cx| {
                    this.uploading = Some(shown);
                    cx.notify();
                });
                if started.is_err() {
                    return;
                }
                let read = cx
                    .background_executor()
                    .spawn(async move { read_capped(&path, api::MAX_MUSIC_UPLOAD_BYTES) })
                    .await;
                let content = match read {
                    Ok(Some(content)) => content,
                    Ok(None) => {
                        cx.update(|cx| Notices::error(t("music_upload_too_large"), cx));
                        break;
                    }
                    Err(error) => {
                        cx.update(|cx| Notices::error(error.to_string(), cx));
                        break;
                    }
                };
                let upload = api::TrackUpload {
                    source_id: id.clone(),
                    filename,
                    content,
                };
                if let Err(error) = link
                    .call(move |api| async move { api.upload_track(upload).await })
                    .await
                {
                    cx.update(|cx| Notices::failed(&error, cx));
                    break;
                }
                uploaded += 1;
            }
            this.update(cx, |this, cx| {
                if uploaded > 0 {
                    Notices::info(t_with("music_uploaded", &[("count", uploaded.into())]), cx);
                    let active = SourcesModel::global(cx)
                        .read(cx)
                        .active()
                        .is_some_and(|source| source.id == this.source_id);
                    if active {
                        JobsModel::global(cx)
                            .update(cx, |jobs, cx| jobs.start(JobKind::LibrarySync, cx));
                    }
                }
                this.quota.update(cx, |quota, cx| quota.refresh(cx));
                this.uploading = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for CloudStorage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.quota.read(cx);
        let usage = match (query.value(), query.error()) {
            (_, Some(error)) => failure(store::human(error), cx),
            (Some(quota), None) => ui::hint(
                t_with(
                    "cloud_storage_usage",
                    &[
                        ("used", mebibytes(quota.used_bytes).into()),
                        ("total", mebibytes(quota.quota_bytes).into()),
                    ],
                ),
                cx,
            ),
            (None, None) => ui::hint(t("cloud_storage_loading"), cx),
        };
        let busy = self.uploading.is_some();
        block(cx)
            .child(usage)
            .when(self.uploads, |column| {
                column.child(
                    h_flex().child(
                        Button::new(SharedString::from(format!("upload-{}", self.source_id)))
                            .custom(style::soft_button(cx.theme().foreground, cx))
                            .small()
                            .icon(icons::icon(icons::PLUS))
                            .label(t("upload_music"))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.pick(cx))),
                    ),
                )
            })
            .when_some(self.uploading.clone(), |column, name| {
                column.child(ui::hint(
                    t_with("uploading_music", &[("name", name.to_string().into())]),
                    cx,
                ))
            })
    }
}

fn mebibytes(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
}

fn is_audio(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            AUDIO
                .iter()
                .any(|audio| extension.eq_ignore_ascii_case(audio))
        })
}

/// The file, or `None` when it is over `limit`: sized from metadata first, and never more than `limit + 1` bytes read.
fn read_capped(path: &Path, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > limit as u64 {
        return Ok(None);
    }
    let mut content = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut content)?;
    Ok((content.len() <= limit).then_some(content))
}

#[cfg(test)]
mod tests {
    use super::read_capped;

    #[test]
    fn read_capped_refuses_a_file_over_the_limit() {
        let path = std::env::temp_dir().join(format!("kopuz-upload-{}", std::process::id()));
        std::fs::write(&path, [7u8; 10]).unwrap();
        assert_eq!(read_capped(&path, 9).unwrap(), None);
        assert_eq!(read_capped(&path, 10).unwrap(), Some(vec![7u8; 10]));
        let _ = std::fs::remove_file(&path);
    }
}

//! Signing a source in again from a browser profile already signed in to its service, or through a new browser window.

use api::{BrowserSession, SourceInfo};
use components::dialogs::{DialogSize, FormDialog, FormFooter};
use components::format::{t, t_with};
use components::{BrowserSessionList, ErrorBanner};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{Disableable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, IntoElement, ParentElement as _, Render, SharedString, Styled,
    Window,
};
use link::Link;
use store::{Notices, SourcesModel};

/// Offer `sessions` for `source`; the store opens this once the daemon has found at least one.
pub fn open(source: SourceInfo, sessions: Vec<BrowserSession>, window: &mut Window, cx: &mut App) {
    let title = SharedString::from(source.name.clone());
    let chooser = cx.new(|_| Reauth {
        source,
        sessions,
        importing: None,
        error: None,
    });
    FormDialog::new(title)
        .size(DialogSize::Wide)
        .footer(FormFooter::Cancel)
        .open(chooser, window, cx);
}

struct Reauth {
    source: SourceInfo,
    sessions: Vec<BrowserSession>,
    importing: Option<String>,
    error: Option<SharedString>,
}

impl Reauth {
    fn take(&mut self, session: BrowserSession, window: &mut Window, cx: &mut Context<Self>) {
        if self.importing.is_some() {
            return;
        }
        self.importing = Some(session.id.clone());
        self.error = None;
        cx.notify();
        let id = self.source.id.clone();
        let call = Link::global(cx)
            .call(move |api| async move { api.import_browser_session(id, session.id).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = call.await;
            this.update_in(cx, |this, window, cx| {
                this.importing = None;
                match result {
                    Ok(_) => {
                        window.close_dialog(cx);
                        SourcesModel::global(cx).update(cx, |sources, cx| sources.refetch(cx));
                        Notices::info(t("signed_in"), cx);
                    }
                    Err(error) => {
                        this.error = Some(t_with(
                            "signin_failed",
                            &[("error", store::human(&error).to_string().into())],
                        ));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// None of the profiles is the account wanted, so the daemon opens its own sign-in window.
    fn browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        store::browser_sign_in(self.source.id.clone(), cx);
    }
}

impl Render for Reauth {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let busy = self.importing.is_some();
        v_flex()
            .w_full()
            .gap_4()
            .child(
                BrowserSessionList::new("reauth-sessions", Some(self.sessions.clone()))
                    .importing(self.importing.clone())
                    .on_use(move |session, window, cx| {
                        this.update(cx, |this, cx| this.take(session, window, cx))
                            .ok();
                    }),
            )
            .when_some(self.error.clone(), |body, message| {
                body.child(ErrorBanner::new(message))
            })
            .child(
                h_flex().child(
                    Button::new("reauth-browser")
                        .outline()
                        .label(t("sign_in_with_browser"))
                        .disabled(busy)
                        .on_click(
                            cx.listener(|this: &mut Self, _, window, cx| this.browser(window, cx)),
                        ),
                ),
            )
    }
}

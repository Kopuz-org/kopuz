//! The username/password sign-in a server asks for when its `SignInKind` is `Password`.

use api::SourceLoginRequest;
use components::ErrorBanner;
use components::dialogs::{DialogSize, FormDialog};
use components::format::{t, t_with};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputContentType, InputState};
use gpui_kit::component::{Disableable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled, Window,
};
use link::Link;
use store::SourcesModel;

/// Open the sign-in dialog for `server_id` on a fresh form, so it works without the settings page open.
pub fn open(server_id: String, service: SharedString, window: &mut Window, cx: &mut gpui_kit::App) {
    let form = cx.new(|cx| LoginForm::new(window, cx));
    form.update(cx, |form, cx| {
        form.reset(server_id, service, window, cx);
    });
    let (title, submit, held) = (form.clone(), form.clone(), form.clone());
    FormDialog::new(SharedString::default())
        .title_with(move |cx| title.read(cx).title())
        .size(DialogSize::Wide)
        .on_submit(move |window, cx| submit.update(cx, |form, cx| form.submit(window, cx)))
        .open(form, window, cx);
    held.update(cx, |form, cx| form.focus(window, cx));
}

pub struct LoginForm {
    server_id: String,
    service: SharedString,
    username: Entity<InputState>,
    password: Entity<InputState>,
    error: Option<SharedString>,
    loading: bool,
}

impl LoginForm {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            server_id: String::new(),
            service: SharedString::default(),
            username: cx.new(|cx| InputState::new(window, cx).placeholder(t("username"))),
            password: cx.new(|cx| InputState::new(window, cx).placeholder(t("password"))),
            error: None,
            loading: false,
        }
    }

    /// Point the form at a server and clear it; a password is never carried over.
    pub fn reset(
        &mut self,
        server_id: String,
        service: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.server_id = server_id;
        self.service = service;
        self.username
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.password
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.error = None;
        self.loading = false;
        cx.notify();
    }

    /// Opening a dialog focuses the dialog itself, so this runs once it is open.
    fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.username
            .update(cx, |state, cx| state.focus(window, cx));
    }

    pub fn title(&self) -> SharedString {
        t_with(
            "login_to_service",
            &[("service", self.service.to_string().into())],
        )
    }

    /// Send the sign-in; it closes the dialog itself on success, so a caller must not close it too.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let request = SourceLoginRequest {
            server_id: self.server_id.clone(),
            username: self.username.read(cx).value().to_string(),
            password: self.password.read(cx).value().to_string(),
        };
        if request.username.is_empty() || request.password.is_empty() {
            self.error = Some(t("username_and_password_required"));
            cx.notify();
            return;
        }
        self.error = None;
        self.loading = true;
        cx.notify();
        let call = Link::global(cx).call(move |api| async move { api.login_source(request).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                this.loading = false;
                cx.notify();
            })
            .ok();
            match result {
                Ok(_) => {
                    cx.update(|window, cx| {
                        SourcesModel::global(cx).update(cx, |sources, cx| sources.refetch(cx));
                        window.close_dialog(cx);
                    })
                    .ok();
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.error = Some(t_with(
                            "login_failed",
                            &[("error", store::human(&error).to_string().into())],
                        ));
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();
    }
}

impl Render for LoginForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loading = self.loading;
        v_flex()
            .w_full()
            .gap_3()
            .child(Input::new(&self.username).disabled(loading))
            .child(
                Input::new(&self.password)
                    .content_type(InputContentType::Password)
                    .mask_toggle()
                    .disabled(loading),
            )
            .when_some(self.error.clone(), |form, message| {
                form.child(ErrorBanner::new(message))
            })
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("cancel-login")
                            .outline()
                            .label(t("cancel"))
                            .disabled(loading)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("submit-login")
                            .primary()
                            .label(if loading { t("logging_in") } else { t("login") })
                            .loading(loading)
                            .disabled(loading)
                            .on_click(cx.listener(|this: &mut Self, _, window, cx| {
                                this.submit(window, cx)
                            })),
                    ),
            )
    }
}

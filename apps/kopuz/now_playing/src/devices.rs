//! Where the active source can play, when it plays somewhere other than this app.

use api::ExternalDevice;
use components::format::t;
use components::{DeviceRow, icons};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    Anchor, App, Context, Hsla, IntoElement, ParentElement as _, Styled, WeakEntity, Window, div,
    px,
};
use store::{Chrome, Link, Notices, PlayerModel, SourcesModel};

use crate::NowPlaying;
use crate::bar::utility;

const PANEL_WIDTH: gpui_kit::Pixels = px(280.);

/// The active source's id, but only while it has devices of its own and is signed in.
fn device_source(cx: &App) -> Option<String> {
    let sources = SourcesModel::global(cx);
    let source = sources.read(cx).active()?.clone();
    (source.capabilities.external_devices && source.authenticated).then_some(source.id)
}

impl NowPlaying {
    pub(crate) fn devices_button(
        &self,
        accent: Hsla,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        device_source(cx)?;
        let elsewhere = PlayerModel::global(cx)
            .read(cx)
            .external()
            .is_some_and(|external| external.device.is_some());
        let devices = self.devices.clone();
        let owner = cx.entity().downgrade();
        let open = self.devices_open;
        let changed = cx.listener(|this, open: &bool, _, cx| this.set_devices_open(*open, cx));

        // The popover wants a `Selectable` trigger, so the button is handed to it inside its frame.
        Some(
            utility("devices", icons::SPEAKER, self.scale(cx))
                .lit(elsewhere)
                .accent(accent)
                .tooltip(t("external_play_on"))
                .host(move |button| {
                    Popover::new("external-devices")
                        .anchor(Anchor::BottomRight)
                        .open(open)
                        .on_open_change(changed)
                        .trigger(button)
                        .content(move |_, _, cx| device_list(devices.clone(), owner.clone(), cx))
                        .into_any_element()
                }),
        )
    }

    fn set_devices_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.devices_open = open;
        if open {
            // The devices popover and the docked panel both want the right edge, so only one holds it.
            Chrome::global(cx).update(cx, |chrome, cx| chrome.set_panel(None, cx));
            self.refresh_devices(cx);
        }
        cx.notify();
    }

    /// A source switch invalidates the list; keep it only while the new source still offers one.
    pub(crate) fn forget_devices(&mut self, cx: &mut Context<Self>) {
        if device_source(cx).is_none() {
            self.devices_open = false;
            self.devices.clear();
        } else if self.devices_open {
            self.refresh_devices(cx);
        }
    }

    fn refresh_devices(&mut self, cx: &mut Context<Self>) {
        let Some(source) = device_source(cx) else {
            self.devices.clear();
            return;
        };
        let call =
            Link::global(cx).call(move |api| async move { api.external_devices(source).await });
        self._devices_fetch = Some(cx.spawn(async move |this, cx| {
            let devices = call.await.unwrap_or_default();
            this.update(cx, |this, cx| {
                this.devices = devices;
                cx.notify();
            })
            .ok();
        }));
    }

    fn select_device(&mut self, device_id: Option<String>, cx: &mut Context<Self>) {
        let Some(source) = device_source(cx) else {
            return;
        };
        let call = Link::global(cx)
            .call(move |api| async move { api.select_external_device(source, device_id).await });
        cx.spawn(async move |this, cx| {
            if let Err(error) = call.await {
                cx.update(|cx| Notices::failed(&error, cx));
            }
            this.update(cx, |this, cx| this.refresh_devices(cx)).ok();
        })
        .detach();
    }
}

fn device_list(
    devices: Vec<ExternalDevice>,
    owner: WeakEntity<NowPlaying>,
    cx: &mut App,
) -> impl IntoElement + use<> {
    let chosen = devices
        .iter()
        .find(|device| device.active)
        .map(|device| device.id.clone());
    let pick = |target: Option<String>| {
        let owner = owner.clone();
        move |_: &mut Window, cx: &mut App| {
            owner
                .update(cx, |this, cx| this.select_device(target.clone(), cx))
                .ok();
        }
    };
    let rows: Vec<_> = devices
        .iter()
        .map(|device| {
            DeviceRow::new(device.id.clone(), device.name.clone())
                .subtitle((!device.kind.is_empty()).then(|| device.kind.clone().into()))
                .chosen(chosen.as_deref() == Some(device.id.as_str()))
                .on_select(pick(Some(device.id.clone())))
        })
        .collect();

    v_flex()
        .w(PANEL_WIDTH)
        .gap_0p5()
        .child(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(t("external_play_on")),
        )
        .child(
            DeviceRow::new("external-this-app", t("external_this_app"))
                .glyph(icons::MUSIC)
                .chosen(chosen.is_none())
                .on_select(pick(None)),
        )
        .children(rows)
}

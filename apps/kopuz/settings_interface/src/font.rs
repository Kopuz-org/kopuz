use std::borrow::Cow;

use anyhow::Context as _;
use gpui_kit::{App, Global, SharedString, Task};
use store::{ConfigModel, Notices};

#[derive(Default)]
struct CustomFont {
    path: String,
    family: Option<SharedString>,
    _load: Option<Task<()>>,
}

impl Global for CustomFont {}

pub(super) fn watch(cx: &mut App) {
    cx.set_global(CustomFont::default());
    cx.observe(&ConfigModel::global(cx), |_, cx| load(cx))
        .detach();
    load(cx);
}

pub(super) fn family(cx: &App) -> Option<SharedString> {
    cx.global::<CustomFont>().family.clone()
}

fn load(cx: &mut App) {
    let path = ConfigModel::global(cx)
        .read(cx)
        .config()
        .ui
        .custom_font_path
        .clone();
    if cx.global::<CustomFont>().path == path {
        return;
    }
    *cx.global_mut::<CustomFont>() = CustomFont {
        path: path.clone(),
        ..Default::default()
    };
    super::apply::resolve(cx);
    if path.is_empty() {
        return;
    }
    let read_path = path.clone();
    let read = cx
        .background_executor()
        .spawn(async move { read_font(&read_path) });
    let task = cx.spawn(async move |cx| {
        let result = read.await;
        cx.update(|cx| {
            if cx.global::<CustomFont>().path != path {
                return;
            }
            let result = result.and_then(|(bytes, family)| {
                cx.text_system().add_fonts(vec![Cow::Owned(bytes)])?;
                Ok(family)
            });
            match result {
                Ok(family) => cx.global_mut::<CustomFont>().family = Some(family.into()),
                Err(error) => {
                    // TODO(i18n): add a custom-font loading error to the shared locale catalog.
                    Notices::error(format!("Could not load the custom font: {error}"), cx);
                }
            }
            super::apply::resolve(cx);
            cx.refresh_windows();
        });
    });
    cx.global_mut::<CustomFont>()._load = Some(task);
}

fn read_font(path: &str) -> anyhow::Result<(Vec<u8>, String)> {
    let bytes = std::fs::read(path).context("the file could not be read")?;
    let mut fonts = fontdb::Database::new();
    fonts.load_font_data(bytes.clone());
    let family = fonts
        .faces()
        .find_map(|face| face.families.first().map(|(family, _)| family.clone()))
        .context("choose a valid OpenType or TrueType font")?;
    Ok((bytes, family))
}

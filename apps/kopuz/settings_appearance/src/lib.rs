//! Settings → General and Appearance, plus the custom theme editor embedded under them.

mod theme_editor;

use components::format::t;
use components::icons;
use components::settings::{
    Choice, SettingRow, SliderRow, on_slider_change, section, set_slider, slider_state, toggle_row,
};
use components::{PathField, PathKind, ThemeGallery, ThemeOption, style};
use config::{
    BackBehavior, BackgroundEdit, LyricsEdit, PlaybackEdit, PlayerBarPosition, Settings,
    SettingsEdit, SettingsLayout, SystemEdit, TitlebarMode, UiEdit, UiStyle,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::slider::SliderState;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled,
    Subscription, Window, div, rems,
};
use store::ConfigModel;
use theme_editor::ThemeEditor;

/// The i18n key each built-in theme is listed under; `kettek16` is a name, not a phrase.
fn theme_label(id: &str) -> SharedString {
    let key = match id {
        "default" => "default_theme",
        "amoled" => "amoled_black",
        "gruvbox" => "gruvbox_material",
        "gruvbox-classic" => "gruvbox_classic",
        "gruvbox-dark-soft" => "gruvbox_dark_soft",
        "catppuccin" => "catppuccin_mocha",
        "ef-night" => "ef_night",
        "ayu-dark" => "ayu_dark",
        "ayu-mirage" => "ayu_mirage",
        "onedarkpro" => "one_dark_pro",
        "kanagawa-dragon" => "kanagawa_dragon",
        "default-light" => "default_light",
        "catppuccin-latte" => "catppuccin_latte",
        "rosepine-dawn" => "rosepine_dawn",
        "everforest-light" => "everforest_light",
        "ayu-light" => "ayu_light",
        "one-light" => "one_light",
        "gruvbox-light" => "gruvbox_light_soft",
        "dracula" | "nord" | "vague" | "osmium" | "everforest" | "rosepine" => id,
        other => return SharedString::from(other.to_string()),
    };
    t(key)
}

/// `text-white/50` on a path the row shows rather than lets you type.
const PATH: f32 = 0.50;
/// `border-white/10` and `bg-black/25` on the build card.
const CARD_BORDER: f32 = 0.10;
const CARD_BG: f32 = 0.25;
/// `text-white/45` on the commit under the version.
const COMMIT: f32 = 0.45;

/// What a whole-percent slider reads as.
fn percent(value: f32) -> SharedString {
    SharedString::from(format!("{}%", value as i32))
}

pub struct AppearanceSettings {
    editor: Entity<ThemeEditor>,
    darkening: Entity<SliderState>,
    blur: Entity<SliderState>,
    lyrics_blur: Entity<SliderState>,
    section: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl AppearanceSettings {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = ConfigModel::global(cx);
        let cfg = config.read(cx).config().clone();
        let darkening =
            cx.new(|_| slider_state(0., 95., 5., cfg.background.cover_art_darkening as f32));
        let blur = cx.new(|_| slider_state(0., 100., 5., cfg.background.cover_art_blur as f32));
        let lyrics_blur =
            cx.new(|_| slider_state(10., 200., 10., cfg.lyrics.lyrics_depth_blur_strength as f32));

        let subscriptions = vec![
            cx.observe_in(&config, window, |this, _, window, cx| {
                this.sync(window, cx);
                cx.notify();
            }),
            on_slider_change(
                &darkening,
                cx,
                ConfigModel::writer(|value: f32| {
                    SettingsEdit::Background(BackgroundEdit::CoverArtDarkening(
                        value.clamp(0., 95.) as u8,
                    ))
                }),
            ),
            on_slider_change(
                &blur,
                cx,
                ConfigModel::writer(|value: f32| {
                    SettingsEdit::Background(BackgroundEdit::CoverArtBlur(
                        value.clamp(0., 100.) as u8
                    ))
                }),
            ),
            on_slider_change(
                &lyrics_blur,
                cx,
                ConfigModel::writer(|value: f32| {
                    SettingsEdit::Lyrics(LyricsEdit::LyricsDepthBlurStrength(
                        value.clamp(10., 200.) as u8,
                    ))
                }),
            ),
        ];

        Self {
            editor: cx.new(|cx| ThemeEditor::new(window, cx)),
            darkening,
            blur,
            lyrics_blur,
            section: None,
            _subscriptions: subscriptions,
        }
    }

    /// The daemon can answer with values this app did not type, so the sliders follow the config.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cfg = ConfigModel::global(cx).read(cx).config().clone();
        set_slider(
            &self.darkening,
            cfg.background.cover_art_darkening as f32,
            window,
            cx,
        );
        set_slider(&self.blur, cfg.background.cover_art_blur as f32, window, cx);
        set_slider(
            &self.lyrics_blur,
            cfg.lyrics.lyrics_depth_blur_strength as f32,
            window,
            cx,
        );
    }

    /// Path settings share one shape: the chosen path, Remove when set, and the picker button.
    fn path_row(
        &self,
        key: &'static str,
        path: String,
        choose: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let locked = ConfigModel::global(cx).read(cx).is_locked(key);
        PathField::new(key, path)
            .kind(PathKind::File)
            .choose(choose)
            .removable(true)
            .locked(locked)
            .on_change(move |value, cx| {
                if let Some(edit) = path_edit(key, value) {
                    ConfigModel::edit(cx, edit)
                }
            })
            .into_any_element()
    }

    fn general(&self, cfg: &Settings, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows = Vec::new();
        push(
            &mut rows,
            toggle_row(
                "auto_check_updates",
                t("auto_check_updates"),
                cfg.system.auto_check_updates,
                ConfigModel::writer(|on| SettingsEdit::System(SystemEdit::AutoCheckUpdates(on))),
            )
            .bind("auto_check_updates", cx)
            .description(t("auto_check_updates_description")),
            cx,
        );
        push(
            &mut rows,
            toggle_row(
                "minimize_to_tray",
                t("minimize_to_tray"),
                cfg.system.minimize_to_tray,
                ConfigModel::writer(|on| SettingsEdit::System(SystemEdit::MinimizeToTray(on))),
            )
            .bind("minimize_to_tray", cx)
            .description(t("minimize_to_tray_description")),
            cx,
        );
        push(
            &mut rows,
            SettingRow::new(t("back_behavior"))
                .bind("back_behavior", cx)
                .description(t("back_behavior_description"))
                .setting(
                    Choice::new(
                        "back_behavior",
                        cfg.playback.back_behavior,
                        vec![
                            (BackBehavior::RewindThenPrev, t("back_behavior_rewind")),
                            (BackBehavior::AlwaysPrev, t("back_behavior_always_prev")),
                        ],
                    )
                    .on_change(ConfigModel::writer(|value| {
                        SettingsEdit::Playback(PlaybackEdit::BackBehavior(value))
                    })),
                ),
            cx,
        );
        rows
    }

    fn appearance(&self, cfg: &Settings, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows = Vec::new();
        push(
            &mut rows,
            SettingRow::new(t("language"))
                .bind("language", cx)
                .setting(self.language(cfg)),
            cx,
        );
        push(
            &mut rows,
            SettingRow::new(t("appearance"))
                .bind("theme", cx)
                .description(t("theme_description"))
                .stacked()
                .setting(self.theme(cfg)),
            cx,
        );
        if cfg!(not(target_os = "android")) && cfg.ui.theme == utils::live_theme::THEME_ID {
            push(
                &mut rows,
                SettingRow::new(t("live_theme_file"))
                    .bind("live_theme_path", cx)
                    .control(self.live_theme_path(cfg, cx)),
                cx,
            );
        }
        push(
            &mut rows,
            toggle_row(
                "cover_art_background",
                t("cover_art_background"),
                cfg.background.cover_art_background,
                ConfigModel::writer(|on| {
                    SettingsEdit::Background(BackgroundEdit::CoverArtBackground(on))
                }),
            )
            .bind("cover_art_background", cx)
            .description(t("cover_art_background_description")),
            cx,
        );
        push(
            &mut rows,
            SettingRow::new(t("custom_background"))
                .bind("custom_background_path", cx)
                .description(t("custom_background_description"))
                .control(self.path_row(
                    "custom_background_path",
                    cfg.background.custom_background_path.clone(),
                    t("choose_image"),
                    cx,
                )),
            cx,
        );
        push(
            &mut rows,
            SettingRow::new(t("custom_font"))
                .bind("custom_font_path", cx)
                .description(t("custom_font_description"))
                .control(self.path_row(
                    "custom_font_path",
                    cfg.ui.custom_font_path.clone(),
                    t("choose_font"),
                    cx,
                )),
            cx,
        );

        if cfg.background.cover_art_background || !cfg.background.custom_background_path.is_empty()
        {
            push(
                &mut rows,
                SettingRow::new(t("cover_art_darkening"))
                    .bind("cover_art_darkening", cx)
                    .setting(SliderRow::new(&self.darkening).readout(percent)),
                cx,
            );
            push(
                &mut rows,
                SettingRow::new(t("cover_art_blur"))
                    .bind("cover_art_blur", cx)
                    .setting(
                        SliderRow::new(&self.blur)
                            .readout(|value| SharedString::from(format!("{}px", value as i32))),
                    ),
                cx,
            );
        }

        push(
            &mut rows,
            toggle_row(
                "lyrics_depth_blur",
                t("lyrics_depth_blur"),
                cfg.lyrics.lyrics_depth_blur,
                ConfigModel::writer(|on| SettingsEdit::Lyrics(LyricsEdit::LyricsDepthBlur(on))),
            )
            .bind("lyrics_depth_blur", cx),
            cx,
        );
        if cfg.lyrics.lyrics_depth_blur {
            push(
                &mut rows,
                SettingRow::new(t("lyrics_depth_blur_strength"))
                    .bind("lyrics_depth_blur_strength", cx)
                    .setting(SliderRow::new(&self.lyrics_blur).readout(percent)),
                cx,
            );
        }

        push(
            &mut rows,
            toggle_row(
                "reduce_animations",
                t("reduce_animations"),
                cfg.ui.reduce_animations,
                ConfigModel::writer(|on| SettingsEdit::Ui(UiEdit::ReduceAnimations(on))),
            )
            .bind("reduce_animations", cx)
            .description(t("reduce_animations_description")),
            cx,
        );
        push(
            &mut rows,
            toggle_row(
                "fullscreen_use_player_bar",
                t("fullscreen_use_player_bar"),
                cfg.ui.fullscreen_use_player_bar,
                ConfigModel::writer(|on| SettingsEdit::Ui(UiEdit::FullscreenUsePlayerBar(on))),
            )
            .bind("fullscreen_use_player_bar", cx),
            cx,
        );
        push(
            &mut rows,
            toggle_row(
                "show_source_toggle",
                t("show_source_toggle"),
                cfg.ui.show_source_toggle,
                ConfigModel::writer(|on| SettingsEdit::Ui(UiEdit::ShowSourceToggle(on))),
            )
            .bind("show_source_toggle", cx)
            .description(t("show_source_toggle_description")),
            cx,
        );
        push(
            &mut rows,
            toggle_row(
                "show_row_images",
                t("show_row_images"),
                cfg.ui.show_row_images,
                ConfigModel::writer(|on| SettingsEdit::Ui(UiEdit::ShowRowImages(on))),
            )
            .bind("show_row_images", cx)
            .description(t("show_row_images_description")),
            cx,
        );
        if cfg!(any(target_os = "linux", target_os = "windows")) {
            push(
                &mut rows,
                SettingRow::new(t("titlebar_mode"))
                    .bind("titlebar_mode", cx)
                    .setting(
                        Choice::new(
                            "titlebar_mode",
                            cfg.ui.titlebar_mode,
                            vec![
                                (TitlebarMode::Custom, t("titlebar_custom")),
                                (TitlebarMode::System, t("titlebar_system")),
                                (TitlebarMode::Off, t("titlebar_off")),
                            ],
                        )
                        .on_change(ConfigModel::writer(|value| {
                            SettingsEdit::Ui(UiEdit::TitlebarMode(value))
                        })),
                    ),
                cx,
            );
        }
        push(
            &mut rows,
            SettingRow::new(t("player_bar_position"))
                .bind("player_bar_position", cx)
                .setting(
                    Choice::new(
                        "player_bar_position",
                        cfg.ui.player_bar_position,
                        vec![
                            (PlayerBarPosition::Bottom, t("position_bottom")),
                            (PlayerBarPosition::Top, t("position_top")),
                        ],
                    )
                    .on_change(ConfigModel::writer(|value| {
                        SettingsEdit::Ui(UiEdit::PlayerBarPosition(value))
                    })),
                ),
            cx,
        );
        // Material 3 always draws the top bar, so the choice would do nothing there.
        if cfg.ui.ui_style != UiStyle::Material3 {
            push(
                &mut rows,
                SettingRow::new(t("settings_layout"))
                    .bind("settings_layout", cx)
                    .setting(
                        Choice::new(
                            "settings_layout",
                            cfg.ui.settings_layout,
                            vec![
                                (SettingsLayout::Cd, t("settings_layout_cd")),
                                (SettingsLayout::TopBar, t("settings_layout_topbar")),
                            ],
                        )
                        .on_change(ConfigModel::writer(|value| {
                            SettingsEdit::Ui(UiEdit::SettingsLayout(value))
                        })),
                    ),
                cx,
            );
        }
        push(
            &mut rows,
            SettingRow::new(t("ui_style")).bind("ui_style", cx).setting(
                Choice::new(
                    "ui_style",
                    cfg.ui.ui_style,
                    vec![
                        (UiStyle::Normal, t("ui_normal")),
                        (UiStyle::Vaxry, t("ui_vaxry")),
                        (UiStyle::Material3, t("ui_material3")),
                    ],
                )
                .on_change(ConfigModel::writer(|value| {
                    SettingsEdit::Ui(UiEdit::UiStyle(value))
                })),
            ),
            cx,
        );
        rows
    }

    /// The file the palette is read from, even when unset: the generator's default is shown then.
    fn live_theme_path(&self, cfg: &Settings, cx: &mut Context<Self>) -> AnyElement {
        let configured = cfg.ui.live_theme_path.clone();
        let resolved = utils::live_theme::resolve_path(&configured)
            .display()
            .to_string();
        h_flex()
            .gap_2()
            .items_center()
            .when(configured.is_empty(), |row| {
                row.child(
                    div()
                        .max_w(rems(13.75))
                        .truncate()
                        .text_xs()
                        .text_color(style::white(PATH, cx))
                        .child(resolved),
                )
            })
            .child(self.path_row("live_theme_path", configured, t("choose_palette"), cx))
            .into_any_element()
    }

    fn language(&self, cfg: &Settings) -> Choice<String> {
        let options = i18n::available_languages()
            .iter()
            .map(|(code, name)| (code.to_string(), SharedString::from(*name)))
            .collect();
        Choice::new("language", cfg.ui.language.clone(), options).on_change(ConfigModel::writer(
            |value| SettingsEdit::Ui(UiEdit::Language(value)),
        ))
    }

    fn theme(&self, cfg: &Settings) -> ThemeGallery {
        theme_gallery("theme", cfg).columns(6)
    }
}

/// Every theme on offer, each with the colours its example is drawn in.
pub fn theme_options(cfg: &Settings) -> Vec<ThemeOption> {
    let mut options: Vec<ThemeOption> = theme::BUILT_IN
        .iter()
        .map(|palette| ThemeOption {
            id: palette.name.to_string(),
            label: theme_label(palette.name),
            colors: palette.into(),
        })
        .collect();
    // System colours follow the cover; the example shows them as they start, before anything plays.
    options.push(ThemeOption {
        id: theme::SYSTEM.to_string(),
        label: t("system_colors"),
        colors: theme::system_colors(None, true),
    });
    // Album art keeps the default's colours under the cover's light, so the default is what its example shows.
    if let Some(palette) = theme::palette("default") {
        options.push(ThemeOption {
            id: theme::ALBUM_ART.to_string(),
            label: t("album_art_gradient"),
            colors: palette.into(),
        });
    }
    // A generator's palette recolours the default, so the default is what its example shows.
    if cfg!(not(target_os = "android"))
        && let Some(palette) = theme::palette("default")
    {
        options.push(ThemeOption {
            id: utils::live_theme::THEME_ID.to_string(),
            label: t("live_theme"),
            colors: palette.into(),
        });
    }
    let mut custom: Vec<ThemeOption> = cfg
        .ui
        .custom_themes
        .iter()
        .filter_map(|(id, custom)| {
            Some(ThemeOption {
                id: id.clone(),
                label: SharedString::from(custom.name.clone()),
                colors: theme::Colors::from_vars(&custom.vars)?,
            })
        })
        .collect();
    custom.sort_by(|left, right| left.label.cmp(&right.label));
    options.extend(custom);
    options
}

/// The theme picker, writing the choice straight to the config.
pub fn theme_gallery(id: &'static str, cfg: &Settings) -> ThemeGallery {
    ThemeGallery::new(id, cfg.ui.theme.clone(), theme_options(cfg)).on_change(ConfigModel::writer(
        |value| SettingsEdit::Ui(UiEdit::Theme(value)),
    ))
}

/// Keep a row unless the settings search is filtering it out.
fn push(rows: &mut Vec<AnyElement>, row: SettingRow, cx: &App) {
    if let Some(row) = row.shown(cx) {
        rows.push(row.into_any_element());
    }
}

/// The version and commit this build came from, copyable as one block for a bug report.
fn build_info(cx: &App) -> impl IntoElement {
    let summary = SharedString::from(utils::build_info::summary());
    h_flex()
        .w_full()
        .mt_4()
        .px_5()
        .py_3()
        .gap_5()
        .justify_between()
        .rounded_xl()
        .border_1()
        .border_color(style::white(CARD_BORDER, cx))
        .bg(style::black(CARD_BG, cx))
        .child(
            v_flex()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(style::white(style::TEXT_STRONG, cx))
                        .child(format!("Kopuz {}", utils::build_info::VERSION)),
                )
                .child(
                    div()
                        .text_xs()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_color(style::white(COMMIT, cx))
                        .child(utils::build_info::COMMIT),
                ),
        )
        .child(
            Button::new("copy-build-info")
                .ghost()
                .small()
                .icon(icons::icon(icons::COPY))
                .tooltip(summary.clone())
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(summary.to_string()))
                }),
        )
}

/// First-run setup sets far more than sources, so its way back in sits under General.
fn run_setup_row(cx: &App) -> impl IntoElement {
    h_flex().w_full().justify_end().child(
        Button::new("run-setup-again")
            .ghost()
            .small()
            .label(t("onboarding_run_again"))
            .text_color(style::white(style::TEXT_SECONDARY, cx))
            .on_click(|_, _, cx| store::SetupModel::start_over(cx)),
    )
}

fn path_edit(key: &str, value: String) -> Option<SettingsEdit> {
    match key {
        "custom_background_path" => Some(SettingsEdit::Background(
            BackgroundEdit::CustomBackgroundPath(value),
        )),
        "custom_font_path" => Some(SettingsEdit::Ui(UiEdit::CustomFontPath(value))),
        "live_theme_path" => Some(SettingsEdit::Ui(UiEdit::LiveThemePath(value))),
        other => {
            tracing::warn!(key = other, "no path setting by that name");
            None
        }
    }
}

impl AppearanceSettings {
    /// Which rail entry is showing; `None` stacks every section.
    pub fn show(&mut self, section: Option<SharedString>, cx: &mut Context<Self>) {
        self.section = section;
        cx.notify();
    }

    fn shows(&self, section: &str) -> bool {
        self.section.as_deref().is_none_or(|shown| shown == section)
    }
}

impl Render for AppearanceSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cfg = ConfigModel::global(cx).read(cx).config().clone();
        // The settings page owns the scroll; this panel is content inside it.
        v_flex()
            .id("appearance-settings")
            .w_full()
            .gap_4()
            .pb_4()
            .pr_2()
            .when(self.shows("general"), |panel| {
                panel.child(
                    v_flex()
                        .gap_2()
                        .child(section(t("general")).children(self.general(&cfg, cx)))
                        .when(components::settings::query(cx).is_none(), |general| {
                            general.child(run_setup_row(cx)).child(build_info(cx))
                        }),
                )
            })
            .when(self.shows("appearance"), |panel| {
                panel.child(section(t("appearance")).children(self.appearance(&cfg, cx)))
            })
            .when(self.shows("appearance"), |panel| {
                panel.child(
                    section(t("theme_editor"))
                        .opaque()
                        .child(div().px_5().py_4().child(self.editor.clone())),
                )
            })
    }
}

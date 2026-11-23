//! Settings → Interface: what the bar keeps in reach, and how the sheet is dressed.

use components::format::t;
use components::settings::{
    SettingRow, SliderRow, choice, on_slider_change, section, slider_state, toggle_row,
};
use gpui_kit::component::slider::SliderState;
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled, Subscription, Window,
};
use store::LayoutModel;
use store::layout::{
    Backdrop, HeroSize, LyricsScale, PlayerSize, QueueStyle, Section as Group, SheetTab, Side,
    TileSize,
};

/// Contribute both sections; call once at startup, after `settings_interface::init`.
pub fn register(cx: &mut App) {
    settings_interface::register(
        "player",
        "interface_player",
        20,
        Some(Group::Player),
        |window, cx| cx.new(|cx| PlayerSettings::new(window, cx)).into(),
        cx,
    );
    settings_interface::register(
        "sheet",
        "interface_sheet",
        21,
        Some(Group::Sheet),
        |window, cx| cx.new(|cx| SheetSettings::new(window, cx)).into(),
        cx,
    );
}

struct PlayerSettings;

impl PlayerSettings {
    fn new(_: &mut Window, _: &mut Context<Self>) -> Self {
        Self
    }
}

impl Render for PlayerSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let player = store::layout(cx).player.clone();
        v_flex()
            .w_full()
            .child(
                section(t("interface_player"))
                    .children(
                        SettingRow::new(t("interface_player_size"))
                            .control(choice(
                                "player-size",
                                &player.size,
                                vec![
                                    (PlayerSize::Small, t("interface_player_cover_small")),
                                    (PlayerSize::Normal, t("ui_normal")),
                                    (PlayerSize::Large, t("interface_player_cover_large")),
                                ],
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| layout.player.size = value)
                                },
                            ))
                            .shown(cx),
                    )
                    .children(
                        SettingRow::new(t("interface_player_cover"))
                            .description(t("interface_player_cover_description"))
                            .control(choice(
                                "player-cover",
                                &player.cover,
                                vec![
                                    (TileSize::Small, t("interface_player_cover_small")),
                                    (TileSize::Medium, t("interface_player_cover_medium")),
                                    (TileSize::Large, t("interface_player_cover_large")),
                                ],
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| layout.player.cover = value)
                                },
                            ))
                            .shown(cx),
                    )
                    .children(
                        toggle_row(
                            "player-shuffle",
                            t("interface_player_shuffle"),
                            player.shuffle,
                            |on, cx| LayoutModel::edit(cx, |layout| layout.player.shuffle = on),
                        )
                        .shown(cx),
                    )
                    .children(
                        toggle_row(
                            "player-repeat",
                            t("interface_player_repeat"),
                            player.repeat,
                            |on, cx| LayoutModel::edit(cx, |layout| layout.player.repeat = on),
                        )
                        .shown(cx),
                    )
                    .children(
                        toggle_row(
                            "player-volume",
                            t("interface_player_volume"),
                            player.volume,
                            |on, cx| LayoutModel::edit(cx, |layout| layout.player.volume = on),
                        )
                        .shown(cx),
                    )
                    .children(
                        toggle_row(
                            "player-queue",
                            t("interface_player_queue"),
                            player.queue,
                            |on, cx| LayoutModel::edit(cx, |layout| layout.player.queue = on),
                        )
                        .shown(cx),
                    )
                    .children(
                        toggle_row(
                            "player-lyrics",
                            t("interface_player_lyrics"),
                            player.lyrics,
                            |on, cx| LayoutModel::edit(cx, |layout| layout.player.lyrics = on),
                        )
                        .shown(cx),
                    )
                    .children(
                        toggle_row(
                            "player-favorite",
                            t("interface_player_favorite"),
                            player.favorite,
                            |on, cx| LayoutModel::edit(cx, |layout| layout.player.favorite = on),
                        )
                        .shown(cx),
                    )
                    .children(
                        toggle_row(
                            "player-quality",
                            t("interface_player_quality"),
                            player.quality,
                            |on, cx| LayoutModel::edit(cx, |layout| layout.player.quality = on),
                        )
                        .description(t("interface_player_quality_description"))
                        .shown(cx),
                    )
                    .children(
                        toggle_row(
                            "player-mini-frameless",
                            t("interface_player_mini_frameless"),
                            player.mini_frameless,
                            |on, cx| {
                                LayoutModel::edit(cx, |layout| layout.player.mini_frameless = on)
                            },
                        )
                        .description(t("interface_player_mini_frameless_description"))
                        .shown(cx),
                    )
                    .children(
                        toggle_row(
                            "player-mini-hides-main",
                            t("interface_player_mini_hides_main"),
                            player.mini_hides_main,
                            |on, cx| {
                                LayoutModel::edit(cx, |layout| layout.player.mini_hides_main = on)
                            },
                        )
                        .description(t("interface_player_mini_hides_main_description"))
                        .shown(cx),
                    )
                    .children(
                        toggle_row(
                            "player-tray-lyrics",
                            t("interface_player_tray_lyrics"),
                            player.tray_lyrics,
                            |on, cx| LayoutModel::edit(cx, |layout| layout.player.tray_lyrics = on),
                        )
                        .description(t("interface_player_tray_lyrics_description"))
                        .shown(cx),
                    ),
            )
            .into_any_element()
    }
}

struct SheetSettings {
    lyrics: Entity<SliderState>,
    _subscriptions: Vec<Subscription>,
}

impl SheetSettings {
    fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let held = store::layout(cx).sheet.lyrics.0;
        let lyrics = cx.new(|_| {
            slider_state(
                f32::from(LyricsScale::MIN),
                f32::from(LyricsScale::MAX),
                10.,
                f32::from(held),
            )
        });
        let moved = on_slider_change(&lyrics, cx, |value, cx| {
            let percent =
                value.clamp(f32::from(LyricsScale::MIN), f32::from(LyricsScale::MAX)) as u16;
            LayoutModel::edit(cx, move |layout| layout.sheet.lyrics = LyricsScale(percent));
        });
        Self {
            lyrics,
            _subscriptions: vec![moved],
        }
    }
}

impl Render for SheetSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sheet = store::layout(cx).sheet.clone();
        v_flex()
            .w_full()
            .child(
                section(t("interface_sheet"))
                    .children(
                        SettingRow::new(t("interface_sheet_tab"))
                            .description(t("interface_sheet_tab_description"))
                            .control(choice(
                                "sheet-tab",
                                &sheet.tab,
                                vec![
                                    (SheetTab::Queue, t("up_next")),
                                    (SheetTab::Lyrics, t("lyrics")),
                                    (SheetTab::Related, t("related")),
                                ],
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| layout.sheet.tab = value)
                                },
                            ))
                            .shown(cx),
                    )
                    .children(
                        SettingRow::new(t("interface_sheet_side"))
                            .description(t("interface_sheet_side_description"))
                            .control(choice(
                                "sheet-side",
                                &sheet.side,
                                vec![
                                    (Side::Right, t("interface_sheet_side_right")),
                                    (Side::Left, t("interface_sheet_side_left")),
                                ],
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| layout.sheet.side = value)
                                },
                            ))
                            .shown(cx),
                    )
                    .children(
                        SettingRow::new(t("interface_queue_side"))
                            .description(t("interface_queue_side_description"))
                            .control(choice(
                                "queue-side",
                                &store::layout(cx).interface.queue_side,
                                vec![
                                    (Side::Left, t("interface_sheet_side_left")),
                                    (Side::Right, t("interface_sheet_side_right")),
                                ],
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| {
                                        layout.interface.queue_side = value
                                    })
                                },
                            ))
                            .shown(cx),
                    )
                    .children(
                        SettingRow::new(t("interface_queue_style"))
                            .description(t("interface_queue_style_description"))
                            .control(choice(
                                "sheet-queue",
                                &sheet.queue,
                                vec![
                                    (QueueStyle::Sections, t("interface_queue_style_sections")),
                                    (QueueStyle::Classic, t("interface_queue_style_classic")),
                                ],
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| layout.sheet.queue = value)
                                },
                            ))
                            .shown(cx),
                    )
                    .children(
                        SettingRow::new(t("interface_lyrics_size"))
                            .description(t("interface_lyrics_size_description"))
                            .control(
                                SliderRow::new(&self.lyrics).readout(|value| {
                                    SharedString::from(format!("{}%", value as i32))
                                }),
                            )
                            .shown(cx),
                    )
                    .children(
                        SettingRow::new(t("interface_sheet_backdrop"))
                            .description(t("interface_sheet_backdrop_description"))
                            .control(choice(
                                "sheet-backdrop",
                                &sheet.backdrop,
                                vec![
                                    (Backdrop::Artwork, t("interface_sheet_backdrop_artwork")),
                                    (Backdrop::Palette, t("interface_sheet_backdrop_palette")),
                                    (Backdrop::Solid, t("interface_sheet_backdrop_solid")),
                                ],
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| layout.sheet.backdrop = value)
                                },
                            ))
                            .shown(cx),
                    )
                    .children(
                        SettingRow::new(t("interface_player_cover"))
                            .description(t("interface_sheet_cover_description"))
                            .control(choice(
                                "sheet-cover",
                                &sheet.cover,
                                vec![
                                    (HeroSize::Small, t("interface_player_cover_small")),
                                    (HeroSize::Medium, t("interface_player_cover_medium")),
                                    (HeroSize::Large, t("interface_player_cover_large")),
                                    (
                                        HeroSize::ExtraLarge,
                                        t("interface_player_cover_extra_large"),
                                    ),
                                ],
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| layout.sheet.cover = value)
                                },
                            ))
                            .shown(cx),
                    ),
            )
            .into_any_element()
    }
}

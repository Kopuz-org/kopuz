use std::rc::Rc;

use api::PlaylistCapability;
use gpui_kit::component::h_flex;
use gpui_kit::{
    AnyElement, App, ClickEvent, IntoElement, ParentElement as _, Pixels, RenderOnce, SharedString,
    Styled as _, Window, px,
};
use store::{Navigator, Offline, PlayerModel, Route, SourcesModel};

use crate::download_toggle::{DownloadToggle, ToggleAct};
use crate::icon_button::{IconButton, IconTone};
use crate::{format::t, icons};

/// `border-white/15 hover:border-white/30`: the catalog-styled page's quieter ring.
const CATALOG_RING: (f32, f32) = (0.15, 0.30);

type Act = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type KeysAct = Rc<dyn Fn(&[String], &mut App)>;
type ShuffleAct = Rc<dyn Fn(bool, &mut App)>;

/// Which page the bar sits on, which sets how big its buttons are and how they are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Look {
    /// A showcase: round 56 play and shuffle, outlined 48 secondaries.
    #[default]
    Hero,
    /// The catalog-styled page: a 64 play among ringed 44s.
    Catalog,
    /// Over a banner's artwork: every button a scrim.
    Banner,
    /// In a page header's action row.
    Compact,
}

struct Metrics {
    play: Pixels,
    round: Pixels,
    small: Pixels,
    round_tone: IconTone,
    small_tone: IconTone,
    ring: Option<(f32, f32)>,
}

impl Look {
    fn metrics(self) -> Metrics {
        match self {
            Look::Hero => Metrics {
                play: px(56.),
                round: px(56.),
                small: px(48.),
                round_tone: IconTone::Quiet,
                small_tone: IconTone::Outline,
                ring: None,
            },
            Look::Catalog => Metrics {
                play: px(64.),
                round: px(44.),
                small: px(44.),
                round_tone: IconTone::Quiet,
                small_tone: IconTone::Quiet,
                ring: Some(CATALOG_RING),
            },
            Look::Banner => Metrics {
                play: px(48.),
                round: px(48.),
                small: px(48.),
                round_tone: IconTone::Scrim,
                small_tone: IconTone::Scrim,
                ring: None,
            },
            Look::Compact => Metrics {
                play: px(40.),
                round: px(40.),
                small: px(40.),
                round_tone: IconTone::Quiet,
                small_tone: IconTone::Outline,
                ring: None,
            },
        }
    }
}

/// What a shuffle press does.
enum Shuffle {
    /// Ask for shuffle flipped, lit while it is on.
    Toggle(ShuffleAct),
    /// Ask for shuffle on and the keys played, for a page with no queue of its own to shuffle.
    Play(KeysAct),
    /// A source's own shuffle, such as a catalog artist's radio.
    With(Act),
}

enum Play {
    Keys(KeysAct),
    Custom(Act),
}

enum Item {
    Play(Play),
    Shuffle(Shuffle),
    Radio(Act),
    Download(ToggleAct),
    Delete(Act),
    Share(Act),
    Artist(Option<String>),
    Extra(AnyElement),
}

/// The bar of round buttons a detail page opens with, in the order the page names them.
#[derive(IntoElement)]
pub struct DetailActions {
    id: SharedString,
    keys: Rc<[String]>,
    look: Look,
    offline: Option<Offline>,
    items: Vec<Item>,
}

impl DetailActions {
    pub fn new(id: impl Into<SharedString>, keys: impl Into<Rc<[String]>>) -> Self {
        Self {
            id: id.into(),
            keys: keys.into(),
            look: Look::Hero,
            offline: None,
            items: Vec::new(),
        }
    }

    pub fn look(mut self, look: Look) -> Self {
        self.look = look;
        self
    }

    /// The download state a page already holds for these keys, so the bar does not rescan them.
    pub fn offline(mut self, offline: Offline) -> Self {
        self.offline = Some(offline);
        self
    }

    /// Play the keys from the top; disabled while there are none.
    pub fn play(mut self, play: impl Fn(&[String], &mut App) + 'static) -> Self {
        self.items.push(Item::Play(Play::Keys(Rc::new(play))));
        self
    }

    /// Play something other than the keys, such as a catalog id only the source can resolve.
    pub fn play_with(
        mut self,
        play: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.items.push(Item::Play(Play::Custom(Rc::new(play))));
        self
    }

    /// The shuffle toggle: lit while on, handed the state a press asks for.
    pub fn shuffle(mut self, set: impl Fn(bool, &mut App) + 'static) -> Self {
        self.items
            .push(Item::Shuffle(Shuffle::Toggle(Rc::new(set))));
        self
    }

    /// Shuffle on, then play the keys; the caller does both.
    pub fn shuffle_play(mut self, play: impl Fn(&[String], &mut App) + 'static) -> Self {
        self.items.push(Item::Shuffle(Shuffle::Play(Rc::new(play))));
        self
    }

    pub fn shuffle_with(
        mut self,
        shuffle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.items
            .push(Item::Shuffle(Shuffle::With(Rc::new(shuffle))));
        self
    }

    /// Start a radio; shown only where the source runs playlist radio.
    pub fn radio(mut self, start: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.items.push(Item::Radio(Rc::new(start)));
        self
    }

    /// Download all or remove all; shown only where the source downloads.
    pub fn download(mut self, toggle: impl Fn(&[String], bool, &mut App) + 'static) -> Self {
        self.items.push(Item::Download(Rc::new(toggle)));
        self
    }

    /// Delete the saved list; shown only where the source holds playlists.
    pub fn delete(mut self, delete: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.items.push(Item::Delete(Rc::new(delete)));
        self
    }

    pub fn share(mut self, share: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.items.push(Item::Share(Rc::new(share)));
        self
    }

    /// A link to the artist by key; nothing is drawn for an album that bills nobody the daemon keyed.
    pub fn artist(mut self, key: Option<String>) -> Self {
        self.items.push(Item::Artist(key));
        self
    }

    /// Anything the page draws itself, placed where it was called.
    pub fn extra(mut self, element: impl IntoElement) -> Self {
        self.items.push(Item::Extra(element.into_any_element()));
        self
    }
}

impl RenderOnce for DetailActions {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let metrics = self.look.metrics();
        let capabilities = SourcesModel::global(cx).read(cx).capabilities();
        let shuffled = PlayerModel::global(cx).read(cx).shuffle();
        let id = self.id;
        let keys = self.keys;
        let ring = |button: IconButton| match metrics.ring {
            Some((rest, hot)) => button.ring(rest, hot),
            None => button,
        };
        let small = |name: &str, icon: &'static str| {
            ring(
                IconButton::new(format!("{id}-{name}"), icon)
                    .size(metrics.small)
                    .tone(metrics.small_tone),
            )
        };
        let mut elements: Vec<AnyElement> = Vec::with_capacity(self.items.len());
        for item in self.items {
            let element = match item {
                Item::Play(play) => {
                    let playable = matches!(play, Play::Custom(_)) || !keys.is_empty();
                    let keys = keys.clone();
                    IconButton::new(format!("{id}-play"), icons::PLAY)
                        .size(metrics.play)
                        .tone(IconTone::Accent)
                        .tooltip(t("play"))
                        .disabled(!playable)
                        .on_click(move |event, window, cx| match &play {
                            Play::Custom(play) => play(event, window, cx),
                            Play::Keys(play) => play(&keys, cx),
                        })
                        .into_any_element()
                }
                Item::Shuffle(mode) => {
                    let button = IconButton::new(format!("{id}-shuffle"), icons::SHUFFLE)
                        .size(metrics.round)
                        .tone(metrics.round_tone);
                    let keys = keys.clone();
                    ring(match mode {
                        Shuffle::Toggle(set) => button
                            .lit(shuffled)
                            .tooltip(match shuffled {
                                true => t("shuffle_on"),
                                false => t("shuffle_off"),
                            })
                            .on_click(move |_, _, cx| set(!shuffled, cx)),
                        Shuffle::Play(play) => button
                            .tooltip(t("shuffle"))
                            .disabled(keys.is_empty())
                            .on_click(move |_, _, cx| play(&keys, cx)),
                        Shuffle::With(shuffle) => button
                            .tooltip(t("shuffle"))
                            .on_click(move |event, window, cx| shuffle(event, window, cx)),
                    })
                    .into_any_element()
                }
                Item::Radio(start) => match capabilities.playlist_radio {
                    true => ring(
                        IconButton::new(format!("{id}-radio"), icons::RADIO)
                            .size(metrics.round)
                            .tone(metrics.round_tone)
                            .tooltip(t("start_radio"))
                            .on_click(move |event, window, cx| start(event, window, cx)),
                    )
                    .into_any_element(),
                    false => continue,
                },
                Item::Download(on_toggle) => match capabilities.downloads {
                    true => {
                        let toggle = DownloadToggle::new(format!("{id}-download"), keys.clone())
                            .diameter(metrics.small)
                            .tone(metrics.small_tone)
                            .on_toggle(move |keys, offline, cx| on_toggle(keys, offline, cx));
                        let toggle = match metrics.ring {
                            Some((rest, hot)) => toggle.ring(rest, hot),
                            None => toggle,
                        };
                        match self.offline {
                            Some(offline) => toggle.offline(offline),
                            None => toggle,
                        }
                        .into_any_element()
                    }
                    false => continue,
                },
                Item::Delete(delete) => match capabilities.playlists != PlaylistCapability::None {
                    true => small("delete", icons::TRASH)
                        .tooltip(t("delete_playlist"))
                        .on_click(move |event, window, cx| delete(event, window, cx))
                        .into_any_element(),
                    false => continue,
                },
                Item::Share(share) => small("share", icons::SHARE)
                    .tooltip(t("share"))
                    .on_click(move |event, window, cx| share(event, window, cx))
                    .into_any_element(),
                Item::Artist(key) => match key {
                    Some(key) => small("artist", icons::MIC)
                        .tooltip(t("go_to_artist"))
                        .on_click(move |_, _, cx| {
                            Navigator::navigate(Route::Artist(key.clone()), cx)
                        })
                        .into_any_element(),
                    None => continue,
                },
                Item::Extra(element) => element,
            };
            elements.push(element);
        }
        let gap = match self.look {
            Look::Hero => px(16.),
            _ => px(12.),
        };
        h_flex()
            .flex_shrink_0()
            .items_center()
            .flex_wrap()
            .gap(gap)
            .children(elements)
    }
}

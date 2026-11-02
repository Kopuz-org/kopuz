use gpui_kit::{App, Context, Entity, SharedString};

/// What the docked panel beside the content shows, when it is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidePanel {
    Queue,
    Lyrics,
    Devices,
}

/// Window chrome state shared by the shell and the pieces that toggle it.
pub struct Chrome {
    panel: Option<SidePanel>,
    sheet: bool,
    mini: bool,
    quick_search: bool,
    search: SharedString,
}

impl Chrome {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(_: &mut Context<Self>) -> Self {
        Self {
            panel: None,
            sheet: false,
            mini: false,
            quick_search: false,
            search: SharedString::default(),
        }
    }

    /// The query the browsing destination is showing; the palette is what puts one here.
    pub fn search(&self) -> &SharedString {
        &self.search
    }

    pub fn set_search(&mut self, query: impl Into<SharedString>, cx: &mut Context<Self>) {
        let query = query.into();
        if query == self.search {
            return;
        }
        self.search = query;
        crate::notify(cx);
    }

    pub fn panel(&self) -> Option<SidePanel> {
        self.panel
    }

    /// Whether the now-playing sheet is up over the page.
    pub fn is_sheet_open(&self) -> bool {
        self.sheet
    }

    /// Whether the mini player has taken over from the full window.
    pub fn is_mini(&self) -> bool {
        self.mini
    }

    pub fn is_quick_search_open(&self) -> bool {
        self.quick_search
    }

    /// Open `panel`, or close it when it is already the one showing.
    pub fn toggle_panel(&mut self, panel: SidePanel, cx: &mut Context<Self>) {
        self.panel = if self.panel == Some(panel) {
            None
        } else {
            Some(panel)
        };
        crate::notify(cx);
    }

    pub fn set_panel(&mut self, panel: Option<SidePanel>, cx: &mut Context<Self>) {
        self.panel = panel;
        crate::notify(cx);
    }

    pub fn set_sheet(&mut self, open: bool, cx: &mut Context<Self>) {
        self.sheet = open;
        crate::notify(cx);
    }

    pub fn toggle_sheet(&mut self, cx: &mut Context<Self>) {
        let open = self.sheet;
        self.set_sheet(!open, cx);
    }

    pub fn set_mini(&mut self, mini: bool, cx: &mut Context<Self>) {
        self.mini = mini;
        crate::notify(cx);
    }

    /// Raise quick search, which is where typing in any search trigger actually goes.
    pub fn open_quick_search(cx: &mut App) {
        Self::global(cx).update(cx, |chrome, cx| chrome.set_quick_search(true, cx));
    }

    pub fn set_quick_search(&mut self, open: bool, cx: &mut Context<Self>) {
        self.quick_search = open;
        crate::notify(cx);
    }

    pub fn is_fullscreen(&self) -> bool {
        self.sheet
    }

    pub fn set_fullscreen(&mut self, open: bool, cx: &mut Context<Self>) {
        self.set_sheet(open, cx);
    }

    pub fn is_compact(&self) -> bool {
        self.mini
    }

    pub fn set_compact(&mut self, mini: bool, cx: &mut Context<Self>) {
        self.set_mini(mini, cx);
    }
}

type Toggle = std::rc::Rc<dyn Fn(&mut App)>;

/// What opens the mini player. `now_playing` owns the window; this is how a command reaches it.
#[derive(Default)]
struct MiniPlayer(Option<Toggle>);

impl gpui_kit::Global for MiniPlayer {}

/// Register the mini player's toggle; call once at startup.
pub fn register_mini_player(toggle: impl Fn(&mut App) + 'static, cx: &mut App) {
    cx.set_global(MiniPlayer(Some(std::rc::Rc::new(toggle))));
}

/// Show or hide the mini player, whichever way this build manages to open one.
pub fn toggle_mini_player(cx: &mut App) {
    let toggle = cx
        .try_global::<MiniPlayer>()
        .and_then(|held| held.0.clone());
    match toggle {
        Some(toggle) => toggle(cx),
        // Without a registered opener the in-place shrink is all there is.
        None => Chrome::global(cx).update(cx, |chrome, cx| {
            let mini = chrome.is_mini();
            chrome.set_mini(!mini, cx);
        }),
    }
}

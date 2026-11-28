use gpui_kit::{App, Context, Entity};

/// The track a sonic path starts from, held between "Start sonic path here" and "Sonic path to here".
pub struct SonicPathStart {
    from: Option<String>,
}

impl SonicPathStart {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(_: &mut Context<Self>) -> Self {
        Self { from: None }
    }

    pub fn from(&self) -> Option<&str> {
        self.from.as_deref()
    }

    pub fn set(&mut self, key: String, cx: &mut Context<Self>) {
        self.from = Some(key);
        crate::notify(cx);
    }

    /// The remembered start, forgotten.
    pub fn take(&mut self, cx: &mut Context<Self>) -> Option<String> {
        let from = self.from.take();
        crate::notify(cx);
        from
    }
}

use gpui_kit::Context;

/// A page held on screen while the one replacing it loads: from now on it draws only what it last drew.
pub trait Freeze: Sized + 'static {
    fn freeze(&mut self, cx: &mut Context<Self>);
}

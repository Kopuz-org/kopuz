//! A pending call that the next one replaces, which is what every search box and filter waits with.

use std::time::Duration;

use gpui_kit::{Context, Task};

#[derive(Default)]
pub struct Debounce(Option<Task<()>>);

impl Debounce {
    /// Run `then` once `delay` passes with no newer call; a newer call drops this one.
    pub fn run<T: 'static>(
        &mut self,
        delay: Duration,
        cx: &mut Context<T>,
        then: impl FnOnce(&mut T, &mut Context<T>) + 'static,
    ) {
        self.0 = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, then).ok();
        }));
    }

    pub fn cancel(&mut self) {
        self.0 = None;
    }
}

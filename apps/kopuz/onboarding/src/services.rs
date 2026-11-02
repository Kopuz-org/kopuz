//! The services Kopuz can tell what you play, as the daemon lists them: Discord, scrobblers and the like.

use gpui_kit::component::v_flex;
use gpui_kit::{AnyElement, Context, IntoElement, ParentElement as _, Styled};

use crate::FirstRun;

impl FirstRun {
    pub(crate) fn services_step(&mut self, _: &mut Context<Self>) -> AnyElement {
        v_flex()
            .w_full()
            .child(self.integrations.clone())
            .into_any_element()
    }
}

//! What the integration plugins turned on offer to run, and running it.

use api::{ActionPlace, ActionTarget, PluginAction, Table};
use gpui_kit::{App, AppContext as _, Context, Entity, Subscription};

use crate::{Link, Notices, Query};

pub struct PluginActions {
    query: Entity<Query<Vec<PluginAction>>>,
    _subscription: Subscription,
}

impl PluginActions {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        // Turning a plugin on, off, installing or removing one changes the list.
        let query = cx.new(|cx| {
            Query::new(
                &[Table::Plugins],
                |api| async move { api.plugin_actions().await },
                cx,
            )
        });
        let subscription = cx.observe(&query, |_, _, cx| cx.notify());
        Self {
            query,
            _subscription: subscription,
        }
    }

    /// The actions offered at `place`, in the order the daemon listed them.
    pub fn at(&self, place: ActionPlace, cx: &App) -> Vec<PluginAction> {
        self.query
            .read(cx)
            .value()
            .map(|actions| {
                actions
                    .iter()
                    .filter(|action| action.places.contains(&place))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Run `action` on `target` with the person's `input`; what the plugin answers
/// is a notice, and a failure says why.
pub fn run(action: &PluginAction, target: ActionTarget, input: Option<String>, cx: &mut App) {
    let (plugin, id) = (action.plugin.clone(), action.id.clone());
    let call = Link::global(cx)
        .call(move |api| async move { api.run_plugin_action(plugin, id, target, input).await });
    cx.spawn(async move |cx| {
        let answer = call.await;
        cx.update(|cx| match answer {
            Ok(Some(message)) => Notices::info(message, cx),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "a plugin action failed");
                Notices::failed(&error, cx);
            }
        });
    })
    .detach();
}

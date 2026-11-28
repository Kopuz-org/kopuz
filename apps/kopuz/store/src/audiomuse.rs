use api::{AUDIOMUSE_INTEGRATION, IntegrationInfo, Table};
use gpui_kit::{App, AppContext as _, Context, Entity, Subscription};

use crate::query::Query;

/// Whether AudioMuse-AI is set up, which gates every sonic-analysis affordance; it is no source's capability.
pub struct AudioMuseModel {
    integrations: Entity<Query<Vec<IntegrationInfo>>>,
    _subscription: Subscription,
}

impl AudioMuseModel {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    /// False until the daemon says so, and on a daemon that lists no integrations.
    pub fn ready(cx: &App) -> bool {
        Self::global(cx).read(cx).is_ready(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        // Saving or clearing an integration invalidates the servers table.
        let integrations = cx.new(|cx| {
            Query::new(
                &[Table::Servers],
                |api| async move { api.integrations().await },
                cx,
            )
        });
        let subscription = cx.observe(&integrations, |_, _, cx| crate::notify(cx));
        Self {
            integrations,
            _subscription: subscription,
        }
    }

    fn is_ready(&self, cx: &App) -> bool {
        self.integrations.read(cx).value().is_some_and(|listed| {
            listed
                .iter()
                .any(|info| info.id == AUDIOMUSE_INTEGRATION && info.configured)
        })
    }
}

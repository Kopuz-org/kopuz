use std::sync::Arc;
use dioxus::prelude::*;
use buttplug_client::ButtplugClient;

#[derive(Clone, Copy)]
pub struct IntifaceState {
    pub client: Signal<Option<Arc<ButtplugClient>>>,
    pub connected: Signal<bool>,
}

pub fn use_intiface_provider() -> IntifaceState {
    let client = use_signal(|| None::<Arc<ButtplugClient>>);
    let connected = use_signal(|| false);
    use_context_provider(|| IntifaceState { client, connected })
}
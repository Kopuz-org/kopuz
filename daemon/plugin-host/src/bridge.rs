//! What an integration may ask of kopuz itself, beyond the network. The host
//! supplies it; this crate only checks the manifest allows each call.

use std::future::Future;
use std::pin::Pin;

use crate::wit::playback;

/// A plain-text reason, shown to the plugin and logged.
pub type Answer<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;

pub trait Bridge: Send + Sync + 'static {
    fn control(&self, command: playback::Command) -> Answer<()>;
    fn queue(&self, keys: Vec<String>, mode: playback::QueueMode) -> Answer<()>;
}

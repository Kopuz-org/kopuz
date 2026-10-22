//! A JavaScript runtime the daemon borrows from its host.
//!
//! Some sources hand out streams whose URLs are only readable after running a
//! script they ship. The daemon runs those itself where it can. Where it
//! cannot -- a platform whose only capable runtime is the one already drawing
//! the UI -- the host registers that runtime here and the daemon uses it,
//! rather than a frontend knowing which source needed it or why.

pub use server::ytmusic::decipher::{SolveRequest, set_engine, webview_channel};

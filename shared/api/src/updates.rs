/// Where the daemon stands on a newer Kopuz release, as its last check found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum UpdateStatus {
    /// No check has found a newer release, or checks are off.
    #[default]
    Current,
    Available(AvailableUpdate),
    Downloading {
        version: String,
        done: u64,
        total: Option<u64>,
    },
    /// Staged: the daemon exits, and the update applies once the app that asked for it has quit too.
    Ready {
        version: String,
    },
    Failed(AvailableUpdate),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub version: String,
    pub release_url: String,
    /// The daemon can replace this install itself; otherwise its package manager owns it, and the release page is the offer.
    pub installable: bool,
}

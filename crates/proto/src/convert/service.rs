use super::*;
use crate::*;

pub fn favorites_to_proto(value: &api::FavoritesView) -> Favorites {
    Favorites {
        refs: value.refs.clone(),
        generation: value.generation,
    }
}

pub fn favorites_from_proto(value: &Favorites) -> api::FavoritesView {
    api::FavoritesView {
        refs: value.refs.clone(),
        generation: value.generation,
    }
}

pub fn job_status_to_proto(value: &api::JobStatus) -> JobStatus {
    JobStatus {
        id: value.id.clone(),
        kind: job_kind_to_proto(value.kind) as i32,
        state: job_state_to_proto(value.state) as i32,
        phase: value.phase.clone(),
        current: value.current,
        total: value.total,
        message: value.message.clone(),
        error: value.error.as_ref().map(error_body_to_proto),
    }
}

pub fn job_status_from_proto(value: &JobStatus) -> api::JobStatus {
    api::JobStatus {
        id: value.id.clone(),
        kind: job_kind_from_proto(value.kind),
        state: job_state_from_proto(value.state),
        phase: value.phase.clone(),
        current: value.current,
        total: value.total,
        message: value.message.clone(),
        error: value.error.as_ref().map(error_body_from_proto),
    }
}

pub fn config_view_to_proto(value: &api::ConfigView) -> ConfigView {
    ConfigView {
        config: Some(config_to_proto(&value.config)),
        locked_keys: value.locked_keys.clone(),
    }
}

pub fn config_view_from_proto(value: &ConfigView) -> api::ConfigView {
    api::ConfigView {
        config: config_from_proto(value.config.as_ref().unwrap_or(&Config::default())),
        locked_keys: value.locked_keys.clone(),
    }
}

pub fn daemon_status_to_proto(value: &api::DaemonStatus) -> DaemonStatus {
    DaemonStatus {
        version: value.version.clone(),
        uptime_secs: value.uptime_secs,
        proto_revision: value.proto_revision,
    }
}

pub fn daemon_status_from_proto(value: &DaemonStatus) -> api::DaemonStatus {
    api::DaemonStatus {
        version: value.version.clone(),
        uptime_secs: value.uptime_secs,
        proto_revision: value.proto_revision,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_survives_the_wire() {
        let status = api::DaemonStatus {
            version: "0.16.2".into(),
            uptime_secs: 42,
            proto_revision: api::WIRE_REVISION,
        };
        assert_eq!(
            daemon_status_from_proto(&daemon_status_to_proto(&status)),
            status
        );
    }

    /// A daemon too old to know the field sends nothing, which must not read as revision 1.
    #[test]
    fn a_silent_daemon_is_revision_zero() {
        let old = DaemonStatus {
            version: "0.16.1".into(),
            uptime_secs: 1,
            ..Default::default()
        };
        assert_eq!(daemon_status_from_proto(&old).proto_revision, 0);
        assert_ne!(api::WIRE_REVISION, 0);
    }
}

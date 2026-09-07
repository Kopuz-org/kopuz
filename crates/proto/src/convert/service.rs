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
        config_json: value.config.to_string(),
        locked_keys: value.locked_keys.clone(),
    }
}

pub fn config_view_from_proto(value: &ConfigView) -> api::ConfigView {
    api::ConfigView {
        config: serde_json::from_str(&value.config_json).unwrap_or(serde_json::Value::Null),
        locked_keys: value.locked_keys.clone(),
    }
}

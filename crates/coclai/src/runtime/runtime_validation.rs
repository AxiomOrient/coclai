use crate::runtime::errors::RuntimeError;
use crate::runtime::state::StateProjectionLimits;

pub(crate) fn validate_runtime_capacities(
    live_channel_capacity: usize,
    server_request_channel_capacity: usize,
    has_event_sink: bool,
    event_sink_channel_capacity: usize,
    rpc_response_timeout: std::time::Duration,
) -> Result<(), RuntimeError> {
    if live_channel_capacity == 0 {
        return Err(RuntimeError::InvalidConfig(
            "live_channel_capacity must be > 0".to_owned(),
        ));
    }
    if server_request_channel_capacity == 0 {
        return Err(RuntimeError::InvalidConfig(
            "server_request_channel_capacity must be > 0".to_owned(),
        ));
    }
    if has_event_sink && event_sink_channel_capacity == 0 {
        return Err(RuntimeError::InvalidConfig(
            "event_sink_channel_capacity must be > 0 when event_sink is configured".to_owned(),
        ));
    }
    if rpc_response_timeout.is_zero() {
        return Err(RuntimeError::InvalidConfig(
            "rpc_response_timeout must be > 0".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_state_projection_limits(
    limits: &StateProjectionLimits,
) -> Result<(), RuntimeError> {
    if limits.max_threads == 0 {
        return Err(RuntimeError::InvalidConfig(
            "state_projection_limits.max_threads must be > 0".to_owned(),
        ));
    }
    if limits.max_turns_per_thread == 0 {
        return Err(RuntimeError::InvalidConfig(
            "state_projection_limits.max_turns_per_thread must be > 0".to_owned(),
        ));
    }
    if limits.max_items_per_turn == 0 {
        return Err(RuntimeError::InvalidConfig(
            "state_projection_limits.max_items_per_turn must be > 0".to_owned(),
        ));
    }
    if limits.max_text_bytes_per_item == 0 {
        return Err(RuntimeError::InvalidConfig(
            "state_projection_limits.max_text_bytes_per_item must be > 0".to_owned(),
        ));
    }
    if limits.max_stdout_bytes_per_item == 0 {
        return Err(RuntimeError::InvalidConfig(
            "state_projection_limits.max_stdout_bytes_per_item must be > 0".to_owned(),
        ));
    }
    if limits.max_stderr_bytes_per_item == 0 {
        return Err(RuntimeError::InvalidConfig(
            "state_projection_limits.max_stderr_bytes_per_item must be > 0".to_owned(),
        ));
    }
    Ok(())
}

use std::process::ExitStatus;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::time::{sleep, Duration};

use crate::errors::RuntimeError;
use crate::state::ConnectionState;

use super::lifecycle::{detach_generation, spawn_connection_generation};
use super::rpc_io::resolve_transport_closed_pending;
use super::state_projection::state_set_connection;
use super::{now_millis, RestartPolicy, RuntimeInner};

pub(super) async fn wait_for_transport_exit(
    inner: &Arc<RuntimeInner>,
) -> Result<Option<ExitStatus>, RuntimeError> {
    let poll_ms = inner.supervisor_cfg.monitor_poll_ms.max(1);
    loop {
        if inner.shutting_down.load(Ordering::Acquire) {
            return Ok(None);
        }

        let exit_status = {
            let mut guard = inner.transport.lock().await;
            let Some(transport) = guard.as_mut() else {
                return Ok(None);
            };
            transport.try_wait_exit()?
        };

        if let Some(status) = exit_status {
            return Ok(Some(status));
        }

        sleep(Duration::from_millis(poll_ms)).await;
    }
}

/// Exponential restart backoff with bounded jitter.
/// Allocation: none. Complexity: O(1).
pub(super) fn compute_restart_delay(
    attempt: u32,
    base_backoff_ms: u64,
    max_backoff_ms: u64,
) -> Duration {
    let exp = attempt.min(20);
    let scaled = base_backoff_ms.saturating_mul(1u64 << exp);
    let base_delay_ms = scaled.min(max_backoff_ms);
    let jitter_cap_ms = (base_delay_ms / 10).min(1_000);
    let jitter_ms = if jitter_cap_ms == 0 {
        0
    } else {
        pseudo_random_u64() % jitter_cap_ms.saturating_add(1)
    };
    Duration::from_millis(base_delay_ms.saturating_add(jitter_ms))
}

/// Lightweight seed source for restart jitter.
/// Allocation: none. Complexity: O(1).
fn pseudo_random_u64() -> u64 {
    let t = now_millis() as u64;
    let mut x = t ^ t.rotate_left(13) ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x << 7;
    x ^= x >> 9;
    x
}

pub(super) async fn supervisor_loop(inner: Arc<RuntimeInner>) {
    let mut restarts = 0u32;

    loop {
        let _exit_status = match wait_for_transport_exit(&inner).await {
            Ok(Some(status)) => status,
            Ok(None) => break,
            Err(_) => {
                inner.initialized.store(false, Ordering::Release);
                resolve_transport_closed_pending(&inner).await;
                state_set_connection(&inner, ConnectionState::Dead);
                break;
            }
        };

        if inner.shutting_down.load(Ordering::Acquire) {
            break;
        }

        inner.initialized.store(false, Ordering::Release);
        let generation = inner.generation.load(Ordering::Acquire);
        detach_generation(&inner).await;

        match inner.supervisor_cfg.restart {
            RestartPolicy::Never => {
                state_set_connection(&inner, ConnectionState::Dead);
                break;
            }
            RestartPolicy::OnCrash {
                max_restarts,
                base_backoff_ms,
                max_backoff_ms,
            } => {
                if restarts >= max_restarts {
                    state_set_connection(&inner, ConnectionState::Dead);
                    break;
                }

                state_set_connection(&inner, ConnectionState::Restarting { generation });
                let delay = compute_restart_delay(restarts, base_backoff_ms, max_backoff_ms);
                restarts = restarts.saturating_add(1);
                sleep(delay).await;

                if inner.shutting_down.load(Ordering::Acquire) {
                    break;
                }

                if spawn_connection_generation(&inner, generation.saturating_add(1))
                    .await
                    .is_err()
                {
                    state_set_connection(&inner, ConnectionState::Dead);
                    break;
                }
            }
        }
    }
}

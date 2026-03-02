use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::time::{sleep, Duration};

use crate::state::ConnectionState;

use super::lifecycle::{detach_generation, spawn_connection_generation};
use super::state_projection::state_set_connection;
use super::{now_millis, RestartPolicy, RuntimeInner};

pub(super) async fn start_supervisor_task(inner: &Arc<RuntimeInner>) {
    let supervisor_inner = Arc::clone(inner);
    let supervisor_task = tokio::spawn(supervisor_loop(supervisor_inner));
    inner
        .tasks
        .supervisor_task
        .lock()
        .await
        .replace(supervisor_task);
}

pub(super) async fn wait_for_transport_close_signal(inner: &Arc<RuntimeInner>) -> bool {
    if inner.counters.shutting_down.load(Ordering::Acquire) {
        return false;
    }

    tokio::select! {
        _ = inner.io.transport_closed_signal.notified() => true,
        _ = inner.io.shutdown_signal.notified() => false,
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
        if !wait_for_transport_close_signal(&inner).await {
            break;
        }

        if inner.counters.shutting_down.load(Ordering::Acquire) {
            break;
        }

        inner.counters.initialized.store(false, Ordering::Release);
        let generation = inner.counters.generation.load(Ordering::Acquire);
        if detach_generation(&inner).await.is_err() {
            state_set_connection(&inner, ConnectionState::Dead);
            break;
        }

        match inner.spec.supervisor_cfg.restart {
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

                if inner.counters.shutting_down.load(Ordering::Acquire) {
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

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

const SINK_LATENCY_BUCKET_UPPER_US: [u64; 8] =
    [100, 250, 500, 1_000, 2_500, 5_000, 10_000, u64::MAX];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeMetricsSnapshot {
    pub uptime_millis: u64,
    pub ingress_total: u64,
    pub ingress_rate_per_sec: f64,
    pub pending_rpc_count: u64,
    pub pending_server_request_count: u64,
    pub event_sink_queue_depth: u64,
    pub event_sink_queue_dropped: u64,
    pub broadcast_send_failed: u64,
    pub sink_write_count: u64,
    pub sink_write_error_count: u64,
    pub sink_latency_avg_micros: f64,
    pub sink_latency_p95_micros: u64,
    pub sink_latency_max_micros: u64,
}

/// Runtime counters used for snapshots and long-run regression checks.
/// All counters are lock-free atomics; hot paths must remain O(1).
pub(crate) struct RuntimeMetrics {
    start_unix_millis: i64,
    ingress_total: AtomicU64,
    pending_rpc_count: AtomicU64,
    pending_server_request_count: AtomicU64,
    event_sink_queue_depth: AtomicU64,
    event_sink_queue_dropped: AtomicU64,
    broadcast_send_failed: AtomicU64,
    sink_write_count: AtomicU64,
    sink_write_error_count: AtomicU64,
    sink_latency_total_micros: AtomicU64,
    sink_latency_max_micros: AtomicU64,
    sink_latency_buckets: [AtomicU64; 8],
}

impl RuntimeMetrics {
    /// Create runtime metrics with fixed zeroed counters.
    /// Allocation: none. Complexity: O(1).
    pub(crate) fn new(start_unix_millis: i64) -> Self {
        Self {
            start_unix_millis,
            ingress_total: AtomicU64::new(0),
            pending_rpc_count: AtomicU64::new(0),
            pending_server_request_count: AtomicU64::new(0),
            event_sink_queue_depth: AtomicU64::new(0),
            event_sink_queue_dropped: AtomicU64::new(0),
            broadcast_send_failed: AtomicU64::new(0),
            sink_write_count: AtomicU64::new(0),
            sink_write_error_count: AtomicU64::new(0),
            sink_latency_total_micros: AtomicU64::new(0),
            sink_latency_max_micros: AtomicU64::new(0),
            sink_latency_buckets: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
        }
    }

    /// Record one inbound message.
    /// Allocation: none. Complexity: O(1).
    pub(crate) fn record_ingress(&self) {
        self.ingress_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment pending RPC count.
    /// Allocation: none. Complexity: O(1).
    pub(crate) fn inc_pending_rpc(&self) {
        self.pending_rpc_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement pending RPC count (saturating).
    /// Allocation: none. Complexity: O(1).
    pub(crate) fn dec_pending_rpc(&self) {
        saturating_dec(&self.pending_rpc_count);
    }

    /// Force pending RPC count to known value.
    /// Allocation: none. Complexity: O(1).
    pub(crate) fn set_pending_rpc_count(&self, count: u64) {
        self.pending_rpc_count.store(count, Ordering::Relaxed);
    }

    /// Increment pending server-request count.
    /// Allocation: none. Complexity: O(1).
    pub(crate) fn inc_pending_server_request(&self) {
        self.pending_server_request_count
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement pending server-request count (saturating).
    /// Allocation: none. Complexity: O(1).
    pub(crate) fn dec_pending_server_request(&self) {
        saturating_dec(&self.pending_server_request_count);
    }

    /// Force pending server-request count to known value.
    /// Allocation: none. Complexity: O(1).
    pub(crate) fn set_pending_server_request_count(&self, count: u64) {
        self.pending_server_request_count
            .store(count, Ordering::Relaxed);
    }

    /// Record one successful sink queue enqueue.
    /// Allocation: none. Complexity: O(1).
    pub(crate) fn inc_event_sink_queue_depth(&self) {
        self.event_sink_queue_depth.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one sink queue dequeue.
    /// Allocation: none. Complexity: O(1).
    pub(crate) fn dec_event_sink_queue_depth(&self) {
        saturating_dec(&self.event_sink_queue_depth);
    }

    /// Record one dropped envelope before sink processing.
    /// Allocation: none. Complexity: O(1).
    pub(crate) fn record_event_sink_drop(&self) {
        self.event_sink_queue_dropped
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record one failed broadcast send.
    /// Allocation: none. Complexity: O(1).
    pub(crate) fn record_broadcast_send_failed(&self) {
        self.broadcast_send_failed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one sink write attempt with elapsed latency.
    /// Allocation: none. Complexity: O(1).
    pub(crate) fn record_sink_write(&self, latency_micros: u64, is_error: bool) {
        self.sink_write_count.fetch_add(1, Ordering::Relaxed);
        if is_error {
            self.sink_write_error_count.fetch_add(1, Ordering::Relaxed);
        }
        self.sink_latency_total_micros
            .fetch_add(latency_micros, Ordering::Relaxed);
        max_update(&self.sink_latency_max_micros, latency_micros);

        let bucket_index = sink_latency_bucket_index(latency_micros);
        self.sink_latency_buckets[bucket_index].fetch_add(1, Ordering::Relaxed);
    }

    /// Build immutable metrics snapshot for observability/reporting.
    /// Allocation: none. Complexity: O(bucket_count).
    pub(crate) fn snapshot(&self, now_unix_millis: i64) -> RuntimeMetricsSnapshot {
        let uptime_millis = if now_unix_millis <= self.start_unix_millis {
            0
        } else {
            (now_unix_millis - self.start_unix_millis) as u64
        };
        let ingress_total = self.ingress_total.load(Ordering::Relaxed);
        let ingress_rate_per_sec = if uptime_millis == 0 {
            0.0
        } else {
            (ingress_total as f64) / ((uptime_millis as f64) / 1_000.0)
        };

        let sink_write_count = self.sink_write_count.load(Ordering::Relaxed);
        let sink_latency_total_micros = self.sink_latency_total_micros.load(Ordering::Relaxed);
        let sink_latency_avg_micros = if sink_write_count == 0 {
            0.0
        } else {
            (sink_latency_total_micros as f64) / (sink_write_count as f64)
        };

        RuntimeMetricsSnapshot {
            uptime_millis,
            ingress_total,
            ingress_rate_per_sec,
            pending_rpc_count: self.pending_rpc_count.load(Ordering::Relaxed),
            pending_server_request_count: self.pending_server_request_count.load(Ordering::Relaxed),
            event_sink_queue_depth: self.event_sink_queue_depth.load(Ordering::Relaxed),
            event_sink_queue_dropped: self.event_sink_queue_dropped.load(Ordering::Relaxed),
            broadcast_send_failed: self.broadcast_send_failed.load(Ordering::Relaxed),
            sink_write_count,
            sink_write_error_count: self.sink_write_error_count.load(Ordering::Relaxed),
            sink_latency_avg_micros,
            sink_latency_p95_micros: self.sink_latency_p95_micros(),
            sink_latency_max_micros: self.sink_latency_max_micros.load(Ordering::Relaxed),
        }
    }

    fn sink_latency_p95_micros(&self) -> u64 {
        let total = self.sink_write_count.load(Ordering::Relaxed);
        if total == 0 {
            return 0;
        }
        let threshold = total.saturating_mul(95).div_ceil(100);
        let mut cumulative = 0u64;
        for (i, upper) in SINK_LATENCY_BUCKET_UPPER_US.iter().enumerate() {
            cumulative =
                cumulative.saturating_add(self.sink_latency_buckets[i].load(Ordering::Relaxed));
            if cumulative >= threshold {
                return *upper;
            }
        }
        u64::MAX
    }
}

fn sink_latency_bucket_index(latency_micros: u64) -> usize {
    for (i, upper) in SINK_LATENCY_BUCKET_UPPER_US.iter().enumerate() {
        if latency_micros <= *upper {
            return i;
        }
    }
    SINK_LATENCY_BUCKET_UPPER_US.len().saturating_sub(1)
}

fn saturating_dec(v: &AtomicU64) {
    let mut current = v.load(Ordering::Relaxed);
    loop {
        if current == 0 {
            return;
        }
        match v.compare_exchange_weak(current, current - 1, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(next) => current = next,
        }
    }
}

fn max_update(v: &AtomicU64, candidate: u64) {
    let mut current = v.load(Ordering::Relaxed);
    while candidate > current {
        match v.compare_exchange_weak(current, candidate, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(next) => current = next,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_computes_p95_from_histogram() {
        let metrics = RuntimeMetrics::new(0);
        for _ in 0..95 {
            metrics.record_sink_write(80, false);
        }
        for _ in 0..5 {
            metrics.record_sink_write(8_000, false);
        }

        let snapshot = metrics.snapshot(2_000);
        assert_eq!(snapshot.sink_write_count, 100);
        assert_eq!(snapshot.sink_latency_p95_micros, 100);
        assert_eq!(snapshot.sink_latency_max_micros, 8_000);
    }

    #[test]
    fn pending_counters_do_not_underflow() {
        let metrics = RuntimeMetrics::new(0);
        metrics.dec_pending_rpc();
        metrics.dec_pending_server_request();
        let snapshot = metrics.snapshot(1_000);
        assert_eq!(snapshot.pending_rpc_count, 0);
        assert_eq!(snapshot.pending_server_request_count, 0);
    }
}

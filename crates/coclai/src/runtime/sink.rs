use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::runtime::core::io_policy::should_flush_after_n_events;
use crate::runtime::errors::SinkError;
use crate::runtime::events::Envelope;

pub type EventSinkFuture<'a> = Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + 'a>>;

const DEFAULT_EVENTS_PER_FLUSH: u64 = 64;

/// Optional event persistence/export hook.
/// Implementations should avoid panics and return `SinkError` on write failures.
pub trait EventSink: Send + Sync + 'static {
    /// Consume one envelope.
    /// Side effects: sink-specific I/O. Complexity depends on implementation.
    fn on_envelope<'a>(&'a self, envelope: &'a Envelope) -> EventSinkFuture<'a>;
}

#[derive(Debug)]
pub struct JsonlFileSink {
    state: Arc<Mutex<JsonlFileSinkState>>,
}

#[derive(Debug)]
struct JsonlFileSinkState {
    file: File,
    pending_writes: u64,
    flush_policy: JsonlFlushPolicy,
}

/// Durability/throughput tradeoff for JSONL sink flushing.
/// - `EveryEvent`: flush each event write (lowest data-at-risk, highest overhead).
/// - `EveryNEvents`: flush after N writes (higher throughput, up to N-1 events buffered in process).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonlFlushPolicy {
    EveryEvent,
    EveryNEvents { events: u64 },
}

impl Default for JsonlFlushPolicy {
    fn default() -> Self {
        Self::EveryNEvents {
            events: DEFAULT_EVENTS_PER_FLUSH,
        }
    }
}

impl JsonlFileSink {
    /// Open or create JSONL sink file in append mode.
    /// Side effects: filesystem open/create. Complexity: O(1).
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, SinkError> {
        Self::open_with_policy(path, JsonlFlushPolicy::default()).await
    }

    /// Open JSONL sink with explicit flush policy.
    /// Side effects: filesystem open/create. Complexity: O(1).
    pub async fn open_with_policy(
        path: impl AsRef<Path>,
        flush_policy: JsonlFlushPolicy,
    ) -> Result<Self, SinkError> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.as_ref())
            .await
            .map_err(|err| SinkError::Io(err.to_string()))?;
        Ok(Self {
            state: Arc::new(Mutex::new(JsonlFileSinkState {
                file,
                pending_writes: 0,
                flush_policy,
            })),
        })
    }

    #[cfg(test)]
    async fn debug_pending_writes(&self) -> u64 {
        self.state.lock().await.pending_writes
    }
}

impl EventSink for JsonlFileSink {
    /// Serialize one envelope and append a trailing newline.
    /// Allocation: one JSON byte vector. Complexity: O(n), n = serialized envelope bytes.
    fn on_envelope<'a>(&'a self, envelope: &'a Envelope) -> EventSinkFuture<'a> {
        Box::pin(async move {
            let mut bytes = serde_json::to_vec(envelope)
                .map_err(|err| SinkError::Serialize(err.to_string()))?;
            bytes.push(b'\n');

            let mut state = self.state.lock().await;
            state
                .file
                .write_all(&bytes)
                .await
                .map_err(|err| SinkError::Io(err.to_string()))?;
            state.pending_writes = state.pending_writes.saturating_add(1);

            if should_flush(state.flush_policy, state.pending_writes) {
                state
                    .file
                    .flush()
                    .await
                    .map_err(|err| SinkError::Io(err.to_string()))?;
                state.pending_writes = 0;
            }
            Ok(())
        })
    }
}

fn should_flush(policy: JsonlFlushPolicy, pending_writes: u64) -> bool {
    match policy {
        JsonlFlushPolicy::EveryEvent => true,
        JsonlFlushPolicy::EveryNEvents { events } => {
            should_flush_after_n_events(pending_writes, events)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;
    use crate::runtime::events::{Direction, MsgKind};

    fn temp_file_path() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("runtime_sink_{nanos}.jsonl"))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn jsonl_file_sink_writes_one_line_per_envelope() {
        let path = temp_file_path();
        let sink = JsonlFileSink::open_with_policy(&path, JsonlFlushPolicy::EveryEvent)
            .await
            .expect("open sink");

        let envelope = Envelope {
            seq: 1,
            ts_millis: 0,
            direction: Direction::Inbound,
            kind: MsgKind::Notification,
            rpc_id: None,
            method: Some(Arc::from("turn/started")),
            thread_id: Some(Arc::from("thr_1")),
            turn_id: Some(Arc::from("turn_1")),
            item_id: None,
            json: Arc::new(
                json!({"method":"turn/started","params":{"threadId":"thr_1","turnId":"turn_1"}}),
            ),
        };

        sink.on_envelope(&envelope).await.expect("write envelope");

        let contents = fs::read_to_string(&path).expect("read sink file");
        let line = contents.trim_end();
        assert!(!line.is_empty(), "sink line must not be empty");
        let parsed: Envelope = serde_json::from_str(line).expect("valid envelope json");
        assert_eq!(parsed.seq, 1);
        assert_eq!(parsed.method.as_deref(), Some("turn/started"));

        let _ = fs::remove_file(path);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn jsonl_file_sink_batches_flush_by_event_count() {
        let path = temp_file_path();
        let sink =
            JsonlFileSink::open_with_policy(&path, JsonlFlushPolicy::EveryNEvents { events: 2 })
                .await
                .expect("open sink");

        let envelope = Envelope {
            seq: 1,
            ts_millis: 0,
            direction: Direction::Inbound,
            kind: MsgKind::Notification,
            rpc_id: None,
            method: Some(Arc::from("turn/started")),
            thread_id: Some(Arc::from("thr_1")),
            turn_id: Some(Arc::from("turn_1")),
            item_id: None,
            json: Arc::new(
                json!({"method":"turn/started","params":{"threadId":"thr_1","turnId":"turn_1"}}),
            ),
        };

        sink.on_envelope(&envelope).await.expect("write #1");
        assert_eq!(sink.debug_pending_writes().await, 1);

        sink.on_envelope(&envelope).await.expect("write #2");
        assert_eq!(sink.debug_pending_writes().await, 0);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn default_flush_policy_is_batched() {
        assert_eq!(
            JsonlFlushPolicy::default(),
            JsonlFlushPolicy::EveryNEvents {
                events: DEFAULT_EVENTS_PER_FLUSH
            }
        );
    }
}

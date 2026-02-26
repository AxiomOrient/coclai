use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::errors::SinkError;
use crate::events::Envelope;

pub type EventSinkFuture<'a> = Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + 'a>>;

/// Optional event persistence/export hook.
/// Implementations should avoid panics and return `SinkError` on write failures.
pub trait EventSink: Send + Sync + 'static {
    /// Consume one envelope.
    /// Side effects: sink-specific I/O. Complexity depends on implementation.
    fn on_envelope<'a>(&'a self, envelope: &'a Envelope) -> EventSinkFuture<'a>;
}

#[derive(Debug)]
pub struct JsonlFileSink {
    file: Arc<Mutex<File>>,
}

impl JsonlFileSink {
    /// Open or create JSONL sink file in append mode.
    /// Side effects: filesystem open/create. Complexity: O(1).
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, SinkError> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.as_ref())
            .await
            .map_err(|err| SinkError::Io(err.to_string()))?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
        })
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

            let mut file = self.file.lock().await;
            file.write_all(&bytes)
                .await
                .map_err(|err| SinkError::Io(err.to_string()))?;
            file.flush()
                .await
                .map_err(|err| SinkError::Io(err.to_string()))?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;
    use crate::events::{Direction, MsgKind};

    fn temp_file_path() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("coclai_runtime_sink_{nanos}.jsonl"))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn jsonl_file_sink_writes_one_line_per_envelope() {
        let path = temp_file_path();
        let sink = JsonlFileSink::open(&path).await.expect("open sink");

        let envelope = Envelope {
            seq: 1,
            ts_millis: 0,
            direction: Direction::Inbound,
            kind: MsgKind::Notification,
            rpc_id: None,
            method: Some("turn/started".to_owned()),
            thread_id: Some("thr_1".to_owned()),
            turn_id: Some("turn_1".to_owned()),
            item_id: None,
            json: json!({"method":"turn/started","params":{"threadId":"thr_1","turnId":"turn_1"}}),
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
}

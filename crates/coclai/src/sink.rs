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
mod tests;

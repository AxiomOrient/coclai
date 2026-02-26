use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

use crate::errors::RuntimeError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StdioProcessSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<PathBuf>,
}

impl StdioProcessSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StdioTransportConfig {
    pub read_channel_capacity: usize,
    pub write_channel_capacity: usize,
}

impl Default for StdioTransportConfig {
    fn default() -> Self {
        Self {
            read_channel_capacity: 1024,
            write_channel_capacity: 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportJoinResult {
    pub exit_status: ExitStatus,
    pub malformed_line_count: u64,
}

pub struct StdioTransport {
    write_tx: Option<mpsc::Sender<Value>>,
    read_rx: Option<mpsc::Receiver<Value>>,
    malformed_line_count: Arc<AtomicU64>,
    reader_task: Option<JoinHandle<std::io::Result<()>>>,
    writer_task: Option<JoinHandle<std::io::Result<()>>>,
    child: Option<Child>,
    child_exit_status: Option<ExitStatus>,
}

impl StdioTransport {
    pub async fn spawn(
        spec: StdioProcessSpec,
        config: StdioTransportConfig,
    ) -> Result<Self, RuntimeError> {
        if config.read_channel_capacity == 0 {
            return Err(RuntimeError::InvalidConfig(
                "read_channel_capacity must be > 0".to_owned(),
            ));
        }
        if config.write_channel_capacity == 0 {
            return Err(RuntimeError::InvalidConfig(
                "write_channel_capacity must be > 0".to_owned(),
            ));
        }

        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }

        for (key, value) in &spec.env {
            command.env(key, value);
        }

        let mut child = command
            .spawn()
            .map_err(|err| RuntimeError::Internal(format!("failed to spawn child: {err}")))?;

        let stdin = child.stdin.take().ok_or_else(|| {
            RuntimeError::Internal("failed to acquire child stdin pipe".to_owned())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            RuntimeError::Internal("failed to acquire child stdout pipe".to_owned())
        })?;

        let (write_tx, write_rx) = mpsc::channel(config.write_channel_capacity);
        let (read_tx, read_rx) = mpsc::channel(config.read_channel_capacity);
        let malformed_line_count = Arc::new(AtomicU64::new(0));
        let malformed_line_count_clone = Arc::clone(&malformed_line_count);

        let reader_task = tokio::spawn(reader_loop(stdout, read_tx, malformed_line_count_clone));
        let writer_task = tokio::spawn(writer_loop(write_rx, stdin));

        Ok(Self {
            write_tx: Some(write_tx),
            read_rx: Some(read_rx),
            malformed_line_count,
            reader_task: Some(reader_task),
            writer_task: Some(writer_task),
            child: Some(child),
            child_exit_status: None,
        })
    }

    pub fn write_tx(&self) -> mpsc::Sender<Value> {
        self.write_tx
            .as_ref()
            .expect("write sender missing from transport")
            .clone()
    }

    pub fn take_read_rx(&mut self) -> Result<mpsc::Receiver<Value>, RuntimeError> {
        self.read_rx.take().ok_or_else(|| {
            RuntimeError::Internal("read receiver already taken from transport".to_owned())
        })
    }

    pub fn malformed_line_count(&self) -> u64 {
        self.malformed_line_count.load(Ordering::Relaxed)
    }

    /// Non-blocking child status probe.
    /// Allocation: none. Complexity: O(1).
    pub fn try_wait_exit(&mut self) -> Result<Option<ExitStatus>, RuntimeError> {
        if let Some(status) = self.child_exit_status {
            return Ok(Some(status));
        }

        let Some(child) = self.child.as_mut() else {
            return Ok(None);
        };
        let status = child
            .try_wait()
            .map_err(|err| RuntimeError::Internal(format!("child try_wait failed: {err}")))?;
        if let Some(status) = status {
            self.child_exit_status = Some(status);
            return Ok(Some(status));
        }
        Ok(None)
    }

    pub async fn join(mut self) -> Result<TransportJoinResult, RuntimeError> {
        let malformed_line_count = self.malformed_line_count();

        drop(self.read_rx.take());
        drop(self.write_tx.take());

        await_io_task(self.writer_task.take(), "writer").await?;
        await_io_task(self.reader_task.take(), "reader").await?;
        let exit_status = wait_child_exit(&mut self).await?;

        Ok(TransportJoinResult {
            exit_status,
            malformed_line_count,
        })
    }

    /// Shutdown path used by runtime.
    /// It closes outbound queue, attempts bounded writer flush,
    /// waits for graceful child exit and then force-kills on timeout, and joins reader.
    /// Allocation: none. Complexity: O(1) control + O(bytes) flush/write drain.
    pub async fn terminate_and_join(
        mut self,
        flush_timeout: Duration,
        terminate_grace: Duration,
    ) -> Result<TransportJoinResult, RuntimeError> {
        let malformed_line_count = self.malformed_line_count();

        drop(self.read_rx.take());
        drop(self.write_tx.take());

        let mut writer_task = self
            .writer_task
            .take()
            .ok_or_else(|| RuntimeError::Internal("writer task missing in transport".to_owned()))?;

        let writer_join = timeout(flush_timeout, &mut writer_task).await;
        let writer_result = match writer_join {
            Ok(joined) => joined,
            Err(_) => {
                // Flush timed out: continue shutdown by terminating child,
                // then rejoin writer to avoid detached background tasks.
                wait_child_exit_with_grace(&mut self, terminate_grace).await?;
                writer_task.await
            }
        }
        .map_err(|err| RuntimeError::Internal(format!("writer task join failed: {err}")))?;

        if let Err(err) = writer_result {
            return Err(RuntimeError::Internal(format!("writer task failed: {err}")));
        }

        let exit_status = wait_child_exit_with_grace(&mut self, terminate_grace).await?;
        await_io_task(self.reader_task.take(), "reader").await?;

        Ok(TransportJoinResult {
            exit_status,
            malformed_line_count,
        })
    }
}

async fn await_io_task(
    task: Option<JoinHandle<std::io::Result<()>>>,
    label: &str,
) -> Result<(), RuntimeError> {
    let Some(task) = task else {
        return Err(RuntimeError::Internal(format!(
            "{label} task missing in transport"
        )));
    };

    let joined = task.await;
    let task_result =
        joined.map_err(|err| RuntimeError::Internal(format!("{label} task join failed: {err}")))?;
    if let Err(err) = task_result {
        return Err(RuntimeError::Internal(format!(
            "{label} task failed: {err}"
        )));
    }
    Ok(())
}

async fn wait_child_exit(transport: &mut StdioTransport) -> Result<ExitStatus, RuntimeError> {
    if let Some(status) = transport.try_wait_exit()? {
        return Ok(status);
    }

    let child = transport
        .child
        .as_mut()
        .ok_or_else(|| RuntimeError::Internal("child handle missing in transport".to_owned()))?;
    let status = child
        .wait()
        .await
        .map_err(|err| RuntimeError::Internal(format!("child wait failed: {err}")))?;
    transport.child_exit_status = Some(status);
    Ok(status)
}

async fn wait_child_exit_with_grace(
    transport: &mut StdioTransport,
    terminate_grace: Duration,
) -> Result<ExitStatus, RuntimeError> {
    if let Some(status) = transport.try_wait_exit()? {
        return Ok(status);
    }

    let child = transport
        .child
        .as_mut()
        .ok_or_else(|| RuntimeError::Internal("child handle missing in transport".to_owned()))?;

    match timeout(terminate_grace, child.wait()).await {
        Ok(waited) => {
            let status = waited
                .map_err(|err| RuntimeError::Internal(format!("child wait failed: {err}")))?;
            transport.child_exit_status = Some(status);
            Ok(status)
        }
        Err(_) => {
            child
                .kill()
                .await
                .map_err(|err| RuntimeError::Internal(format!("child kill failed: {err}")))?;
            let status = child.wait().await.map_err(|err| {
                RuntimeError::Internal(format!("child wait after kill failed: {err}"))
            })?;
            transport.child_exit_status = Some(status);
            Ok(status)
        }
    }
}

/// Reader loop: one line -> one JSON parse attempt.
/// Allocation: one reusable String buffer per task. Complexity: O(line_length) per line.
async fn reader_loop(
    stdout: ChildStdout,
    inbound_tx: mpsc::Sender<Value>,
    malformed_line_count: Arc<AtomicU64>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stdout);
    let mut line = String::with_capacity(4096);

    loop {
        line.clear();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            break;
        }

        let raw = line.trim_end_matches(['\n', '\r']);
        if raw.is_empty() {
            continue;
        }

        match serde_json::from_str::<Value>(raw) {
            Ok(json) => {
                if inbound_tx.send(json).await.is_err() {
                    break;
                }
            }
            Err(_) => {
                malformed_line_count.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    Ok(())
}

/// Writer loop: single serialization/write path into child stdin.
/// Allocation: one reusable byte buffer per task. Complexity: O(frame_size) per message.
async fn writer_loop(
    mut outbound_rx: mpsc::Receiver<Value>,
    mut stdin: ChildStdin,
) -> std::io::Result<()> {
    let mut frame = Vec::<u8>::with_capacity(4096);

    while let Some(json) = outbound_rx.recv().await {
        frame.clear();

        serde_json::to_writer(&mut frame, &json).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to serialize outbound json: {err}"),
            )
        })?;
        frame.push(b'\n');

        if let Err(err) = stdin.write_all(&frame).await {
            if err.kind() == std::io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(err);
        }
    }

    if let Err(err) = stdin.flush().await {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            return Ok(());
        }
        return Err(err);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use tokio::time::timeout;

    use super::*;

    fn shell_spec(script: &str) -> StdioProcessSpec {
        let mut spec = StdioProcessSpec::new("sh");
        spec.args = vec!["-c".to_owned(), script.to_owned()];
        spec
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_rejects_zero_capacity_channels() {
        let err = match StdioTransport::spawn(
            shell_spec("cat"),
            StdioTransportConfig {
                read_channel_capacity: 0,
                write_channel_capacity: 16,
            },
        )
        .await
        {
            Ok(_) => panic!("must reject zero read channel capacity"),
            Err(err) => err,
        };
        assert!(matches!(err, RuntimeError::InvalidConfig(_)));

        let err = match StdioTransport::spawn(
            shell_spec("cat"),
            StdioTransportConfig {
                read_channel_capacity: 16,
                write_channel_capacity: 0,
            },
        )
        .await
        {
            Ok(_) => panic!("must reject zero write channel capacity"),
            Err(err) => err,
        };
        assert!(matches!(err, RuntimeError::InvalidConfig(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn writer_and_reader_roundtrip() {
        let mut transport =
            StdioTransport::spawn(shell_spec("cat"), StdioTransportConfig::default())
                .await
                .expect("spawn");
        let mut read_rx = transport.take_read_rx().expect("take rx");
        let write_tx = transport.write_tx();

        write_tx
            .send(json!({"method":"ping","params":{"n":1}}))
            .await
            .expect("send #1");
        write_tx
            .send(json!({"method":"pong","params":{"n":2}}))
            .await
            .expect("send #2");
        drop(write_tx);

        let first = timeout(Duration::from_secs(2), read_rx.recv())
            .await
            .expect("recv timeout #1")
            .expect("stream closed #1");
        let second = timeout(Duration::from_secs(2), read_rx.recv())
            .await
            .expect("recv timeout #2")
            .expect("stream closed #2");

        assert_eq!(first["method"], "ping");
        assert_eq!(second["method"], "pong");

        drop(read_rx);
        let joined = transport.join().await.expect("join");
        assert!(joined.exit_status.success());
        assert_eq!(joined.malformed_line_count, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reader_skips_malformed_lines() {
        let script =
            r#"printf '%s\n' '{"method":"ok"}' 'not-json' '{"id":1,"result":{}}' '{broken'"#;
        let mut transport =
            StdioTransport::spawn(shell_spec(script), StdioTransportConfig::default())
                .await
                .expect("spawn");
        let mut read_rx = transport.take_read_rx().expect("take rx");

        let mut parsed = Vec::new();
        while let Some(msg) = timeout(Duration::from_secs(2), read_rx.recv())
            .await
            .expect("recv timeout")
        {
            parsed.push(msg);
        }

        assert_eq!(parsed.len(), 2);
        assert_eq!(transport.malformed_line_count(), 2);

        drop(read_rx);
        let joined = transport.join().await.expect("join");
        assert!(joined.exit_status.success());
        assert_eq!(joined.malformed_line_count, 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reader_survives_100k_lines_stream() {
        let script = r#"
i=0
while [ "$i" -lt 100000 ]; do
  printf '{"method":"tick","params":{"n":%s}}\n' "$i"
  i=$((i+1))
done
"#;
        let mut transport =
            StdioTransport::spawn(shell_spec(script), StdioTransportConfig::default())
                .await
                .expect("spawn");
        let mut read_rx = transport.take_read_rx().expect("take rx");

        let mut count = 0usize;
        while let Some(_msg) = timeout(Duration::from_secs(20), read_rx.recv())
            .await
            .expect("recv timeout")
        {
            count += 1;
        }

        assert_eq!(count, 100_000);
        assert_eq!(transport.malformed_line_count(), 0);

        drop(read_rx);
        let joined = transport.join().await.expect("join");
        assert!(joined.exit_status.success());
        assert_eq!(joined.malformed_line_count, 0);
    }
}

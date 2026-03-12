use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

use crate::runtime::core::io_policy::{
    normalize_text_tail, trim_ascii_line_endings, trim_tail_bytes, validate_positive_capacity,
};
use crate::runtime::errors::RuntimeError;

const DEFAULT_MAX_INBOUND_FRAME_BYTES: usize = 1024 * 1024;
const DEFAULT_STDERR_TAIL_MAX_BYTES: usize = 16 * 1024;

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
    pub max_inbound_frame_bytes: usize,
    pub stderr_tail_max_bytes: usize,
}

impl Default for StdioTransportConfig {
    fn default() -> Self {
        Self {
            read_channel_capacity: 1024,
            write_channel_capacity: 1024,
            max_inbound_frame_bytes: DEFAULT_MAX_INBOUND_FRAME_BYTES,
            stderr_tail_max_bytes: DEFAULT_STDERR_TAIL_MAX_BYTES,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportJoinResult {
    pub exit_status: ExitStatus,
    pub malformed_line_count: u64,
    pub stderr_tail: Option<String>,
}

pub struct StdioTransport {
    write_tx: Option<mpsc::Sender<Value>>,
    read_rx: Option<mpsc::Receiver<Value>>,
    malformed_line_count: Arc<AtomicU64>,
    stderr_diagnostics: Arc<StderrDiagnostics>,
    reader_task: Option<JoinHandle<std::io::Result<()>>>,
    writer_task: Option<JoinHandle<std::io::Result<()>>>,
    stderr_task: Option<JoinHandle<std::io::Result<()>>>,
    child: Option<Child>,
    child_exit_status: Option<ExitStatus>,
}

#[derive(Default)]
struct StderrDiagnostics {
    tail: Mutex<Vec<u8>>,
}

impl StderrDiagnostics {
    fn append_chunk(&self, chunk: &[u8], max_tail_bytes: usize) {
        let mut tail = match self.tail.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        tail.extend_from_slice(chunk);
        trim_tail_bytes(&mut tail, max_tail_bytes);
    }

    fn snapshot(&self) -> Option<String> {
        let tail = match self.tail.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        normalize_text_tail(&tail)
    }

    fn has_data(&self) -> bool {
        let tail = match self.tail.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        !tail.is_empty()
    }
}

impl StdioTransport {
    pub async fn spawn(
        spec: StdioProcessSpec,
        config: StdioTransportConfig,
    ) -> Result<Self, RuntimeError> {
        validate_positive_capacity("read_channel_capacity", config.read_channel_capacity)?;
        validate_positive_capacity("write_channel_capacity", config.write_channel_capacity)?;
        validate_positive_capacity("max_inbound_frame_bytes", config.max_inbound_frame_bytes)?;
        validate_positive_capacity("stderr_tail_max_bytes", config.stderr_tail_max_bytes)?;

        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

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
        let stderr = child.stderr.take().ok_or_else(|| {
            RuntimeError::Internal("failed to acquire child stderr pipe".to_owned())
        })?;

        let (write_tx, write_rx) = mpsc::channel(config.write_channel_capacity);
        let (read_tx, read_rx) = mpsc::channel(config.read_channel_capacity);
        let malformed_line_count = Arc::new(AtomicU64::new(0));
        let malformed_line_count_clone = Arc::clone(&malformed_line_count);
        let stderr_diagnostics = Arc::new(StderrDiagnostics::default());
        let stderr_diagnostics_clone = Arc::clone(&stderr_diagnostics);

        let reader_task = tokio::spawn(reader_loop(
            stdout,
            read_tx,
            malformed_line_count_clone,
            config.max_inbound_frame_bytes,
        ));
        let writer_task = tokio::spawn(writer_loop(write_rx, stdin));
        let stderr_task = tokio::spawn(stderr_loop(
            stderr,
            stderr_diagnostics_clone,
            config.stderr_tail_max_bytes,
        ));

        Ok(Self {
            write_tx: Some(write_tx),
            read_rx: Some(read_rx),
            malformed_line_count,
            stderr_diagnostics,
            reader_task: Some(reader_task),
            writer_task: Some(writer_task),
            stderr_task: Some(stderr_task),
            child: Some(child),
            child_exit_status: None,
        })
    }

    pub fn write_tx(&self) -> Result<mpsc::Sender<Value>, RuntimeError> {
        self.write_tx
            .as_ref()
            .cloned()
            .ok_or_else(|| RuntimeError::Internal("write sender missing from transport".to_owned()))
    }

    pub fn take_read_rx(&mut self) -> Result<mpsc::Receiver<Value>, RuntimeError> {
        self.read_rx.take().ok_or_else(|| {
            RuntimeError::Internal("read receiver already taken from transport".to_owned())
        })
    }

    pub fn malformed_line_count(&self) -> u64 {
        self.malformed_line_count.load(Ordering::Relaxed)
    }

    /// Latest child stderr tail snapshot, if any bytes have been observed.
    /// Allocation: one String clone. Complexity: O(n), n = stored stderr tail bytes.
    pub fn stderr_tail_snapshot(&self) -> Option<String> {
        self.stderr_diagnostics.snapshot()
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

        await_io_task(
            self.writer_task.take(),
            "writer",
            self.stderr_diagnostics.as_ref(),
        )
        .await?;
        await_io_task(
            self.reader_task.take(),
            "reader",
            self.stderr_diagnostics.as_ref(),
        )
        .await?;
        let exit_status = wait_child_exit(&mut self).await?;
        await_io_task(
            self.stderr_task.take(),
            "stderr-reader",
            self.stderr_diagnostics.as_ref(),
        )
        .await?;

        Ok(TransportJoinResult {
            exit_status,
            malformed_line_count,
            stderr_tail: self.stderr_diagnostics.snapshot(),
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
        .map_err(|err| {
            runtime_internal_with_stderr(
                format!("writer task join failed: {err}"),
                self.stderr_diagnostics.as_ref(),
            )
        })?;

        if let Err(err) = writer_result {
            return Err(runtime_internal_with_stderr(
                format!("writer task failed: {err}"),
                self.stderr_diagnostics.as_ref(),
            ));
        }

        let exit_status = wait_child_exit_with_grace(&mut self, terminate_grace).await?;
        await_io_task(
            self.reader_task.take(),
            "reader",
            self.stderr_diagnostics.as_ref(),
        )
        .await?;
        await_io_task(
            self.stderr_task.take(),
            "stderr-reader",
            self.stderr_diagnostics.as_ref(),
        )
        .await?;

        Ok(TransportJoinResult {
            exit_status,
            malformed_line_count,
            stderr_tail: self.stderr_diagnostics.snapshot(),
        })
    }
}

async fn await_io_task(
    task: Option<JoinHandle<std::io::Result<()>>>,
    label: &str,
    stderr_diagnostics: &StderrDiagnostics,
) -> Result<(), RuntimeError> {
    let Some(task) = task else {
        return Err(runtime_internal_with_stderr(
            format!("{label} task missing in transport"),
            stderr_diagnostics,
        ));
    };

    let joined = task.await;
    let task_result = joined.map_err(|err| {
        runtime_internal_with_stderr(
            format!("{label} task join failed: {err}"),
            stderr_diagnostics,
        )
    })?;
    if let Err(err) = task_result {
        return Err(runtime_internal_with_stderr(
            format!("{label} task failed: {err}"),
            stderr_diagnostics,
        ));
    }
    Ok(())
}

fn runtime_internal_with_stderr(
    message: String,
    stderr_diagnostics: &StderrDiagnostics,
) -> RuntimeError {
    if stderr_diagnostics.has_data() {
        RuntimeError::Internal(format!("{message}; child stderr tail captured"))
    } else {
        RuntimeError::Internal(message)
    }
}

async fn wait_child_exit(transport: &mut StdioTransport) -> Result<ExitStatus, RuntimeError> {
    wait_child_exit_inner(transport, None).await
}

async fn wait_child_exit_with_grace(
    transport: &mut StdioTransport,
    terminate_grace: Duration,
) -> Result<ExitStatus, RuntimeError> {
    wait_child_exit_inner(transport, Some(terminate_grace)).await
}

async fn wait_child_exit_inner(
    transport: &mut StdioTransport,
    terminate_grace: Option<Duration>,
) -> Result<ExitStatus, RuntimeError> {
    if let Some(status) = transport.try_wait_exit()? {
        return Ok(status);
    }

    let child = transport.child.as_mut().ok_or_else(|| {
        runtime_internal_with_stderr(
            "child handle missing in transport".to_owned(),
            transport.stderr_diagnostics.as_ref(),
        )
    })?;

    let status = match terminate_grace {
        None => child.wait().await.map_err(|err| {
            runtime_internal_with_stderr(
                format!("child wait failed: {err}"),
                transport.stderr_diagnostics.as_ref(),
            )
        })?,
        Some(grace) => match timeout(grace, child.wait()).await {
            Ok(waited) => waited.map_err(|err| {
                runtime_internal_with_stderr(
                    format!("child wait failed: {err}"),
                    transport.stderr_diagnostics.as_ref(),
                )
            })?,
            Err(_) => {
                child.kill().await.map_err(|err| {
                    runtime_internal_with_stderr(
                        format!("child kill failed: {err}"),
                        transport.stderr_diagnostics.as_ref(),
                    )
                })?;
                child.wait().await.map_err(|err| {
                    runtime_internal_with_stderr(
                        format!("child wait after kill failed: {err}"),
                        transport.stderr_diagnostics.as_ref(),
                    )
                })?
            }
        },
    };
    transport.child_exit_status = Some(status);
    Ok(status)
}

/// Reader loop: one line -> one JSON parse attempt.
/// Allocation: one reusable byte buffer per task. Complexity: O(line_length) per line.
async fn reader_loop(
    stdout: ChildStdout,
    inbound_tx: mpsc::Sender<Value>,
    malformed_line_count: Arc<AtomicU64>,
    max_inbound_frame_bytes: usize,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::<u8>::with_capacity(4096);

    loop {
        line.clear();
        let read = {
            let mut limited_reader = (&mut reader).take((max_inbound_frame_bytes + 1) as u64);
            limited_reader.read_until(b'\n', &mut line).await?
        };
        if read == 0 {
            break;
        }

        if line.len() > max_inbound_frame_bytes {
            malformed_line_count.fetch_add(1, Ordering::Relaxed);
            if !line.ends_with(b"\n") {
                discard_until_newline(&mut reader).await?;
            }
            continue;
        }

        let raw = trim_ascii_line_endings(line.as_slice());
        if raw.is_empty() {
            continue;
        }

        match serde_json::from_slice::<Value>(raw) {
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

async fn discard_until_newline(reader: &mut BufReader<ChildStdout>) -> std::io::Result<()> {
    loop {
        let (consume_len, found_newline) = {
            let buf = reader.fill_buf().await?;
            if buf.is_empty() {
                return Ok(());
            }
            match buf.iter().position(|byte| *byte == b'\n') {
                Some(pos) => (pos + 1, true),
                None => (buf.len(), false),
            }
        };
        reader.consume(consume_len);
        if found_newline {
            return Ok(());
        }
    }
}

async fn stderr_loop(
    stderr: ChildStderr,
    diagnostics: Arc<StderrDiagnostics>,
    max_tail_bytes: usize,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stderr);
    let mut chunk = [0u8; 4096];

    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        diagnostics.append_chunk(&chunk[..read], max_tail_bytes);
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
                ..StdioTransportConfig::default()
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
                ..StdioTransportConfig::default()
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
        let write_tx = transport.write_tx().expect("take tx");

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
        assert!(joined.stderr_tail.is_none());
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
        assert!(joined.stderr_tail.is_none());
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
        assert!(joined.stderr_tail.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reader_drops_oversized_frame_and_recovers_next_frame() {
        let script = r#"
long=$(head -c 2048 </dev/zero | tr '\0' 'a')
printf '{"method":"%s"}\n' "$long"
printf '{"id":1,"result":{"ok":true}}\n'
"#;
        let mut transport = StdioTransport::spawn(
            shell_spec(script),
            StdioTransportConfig {
                max_inbound_frame_bytes: 256,
                ..StdioTransportConfig::default()
            },
        )
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

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["id"], 1);
        assert_eq!(transport.malformed_line_count(), 1);

        drop(read_rx);
        let joined = transport.join().await.expect("join");
        assert!(joined.exit_status.success());
        assert_eq!(joined.malformed_line_count, 1);
        assert!(joined.stderr_tail.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn join_exposes_child_stderr_tail_for_diagnostics() {
        let script = r#"
printf 'diag-line-1\n' >&2
printf 'diag-line-2\n' >&2
"#;
        let mut transport = StdioTransport::spawn(
            shell_spec(script),
            StdioTransportConfig {
                stderr_tail_max_bytes: 128,
                ..StdioTransportConfig::default()
            },
        )
        .await
        .expect("spawn");
        let read_rx = transport.take_read_rx().expect("take rx");
        drop(read_rx);

        let joined = transport.join().await.expect("join");
        assert!(joined.exit_status.success());
        let stderr_tail = joined.stderr_tail.expect("stderr tail must be captured");
        assert!(stderr_tail.contains("diag-line-1"));
        assert!(stderr_tail.contains("diag-line-2"));
    }

    #[test]
    fn runtime_internal_with_stderr_redacts_tail_contents() {
        let diagnostics = StderrDiagnostics::default();
        diagnostics.append_chunk(b"secret-token\n", 128);

        let RuntimeError::Internal(message) =
            runtime_internal_with_stderr("transport failed".to_owned(), &diagnostics)
        else {
            panic!("expected internal runtime error");
        };

        assert!(message.contains("transport failed"));
        assert!(message.contains("child stderr tail captured"));
        assert!(!message.contains("secret-token"));
    }
}

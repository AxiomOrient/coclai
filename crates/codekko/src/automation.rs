//! Minimal session-scoped automation above [`crate::runtime::Session`].
//!
//! V1 keeps the boundary intentionally small:
//! - automate one prepared `Session`
//! - schedule with absolute `SystemTime` boundaries plus one fixed `Duration`
//! - require `every > Duration::ZERO`
//! - run at most one prompt turn at a time
//! - collapse missed ticks into one next eligible run
//! - stop permanently on `stop()`, `stop_at`, `max_runs`, closed session, or any
//!   [`crate::runtime::PromptRunError`]
//!
//! Non-goals for this module:
//! - no cron or human-time parsing
//! - no session creation or resume orchestration
//! - no persistence or restart recovery
//! - no retry or downgrade policy for prompt failures

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;

use crate::runtime::{PromptRunError, PromptRunResult, Session};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationSpec {
    /// Prompt reused for every scheduled turn.
    pub prompt: String,
    /// First eligible run time. `None` starts immediately.
    pub start_at: Option<SystemTime>,
    /// Fixed interval between due times. Must be greater than zero.
    pub every: Duration,
    /// Exclusive upper bound for starting new runs.
    pub stop_at: Option<SystemTime>,
    /// Optional hard cap on completed runs. `Some(0)` stops immediately.
    pub max_runs: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationStatus {
    pub thread_id: String,
    pub runs_completed: u32,
    pub next_due_at: Option<SystemTime>,
    pub last_started_at: Option<SystemTime>,
    pub last_finished_at: Option<SystemTime>,
    pub state: AutomationState,
    pub last_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationState {
    Waiting,
    Running,
    Stopped,
    Failed,
}

impl AutomationState {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Failed)
    }
}

pub struct AutomationHandle {
    shared: Arc<SharedState>,
    join: Mutex<Option<JoinHandle<AutomationStatus>>>,
}

impl AutomationHandle {
    pub async fn stop(&self) {
        self.shared.stop_requested.store(true, Ordering::Release);
        self.shared.stop_notify.notify_waiters();
    }

    pub async fn wait(self) -> AutomationStatus {
        let join = self.join.lock().await.take();
        match join {
            Some(join) => match join.await {
                Ok(status) => status,
                Err(err) => {
                    let mut status = self.shared.snapshot().await;
                    status.state = AutomationState::Failed;
                    status.next_due_at = None;
                    status.last_error = Some(format!("automation task join error: {err}"));
                    status
                }
            },
            None => self.shared.snapshot().await,
        }
    }

    pub async fn status(&self) -> AutomationStatus {
        self.shared.snapshot().await
    }
}

/// Start one background automation loop for one prepared session.
///
/// Invalid specs do not panic: for example, `every == Duration::ZERO` returns a handle whose
/// status is immediately terminal with `AutomationState::Failed`.
pub fn spawn(session: Session, spec: AutomationSpec) -> AutomationHandle {
    spawn_runner(Arc::new(SessionRunner(session)), spec)
}

type TurnFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PromptRunResult, PromptRunError>> + Send + 'a>>;

trait TurnRunner: Send + Sync + 'static {
    fn thread_id(&self) -> &str;
    fn is_closed(&self) -> bool;
    fn run_prompt<'a>(&'a self, prompt: &'a str) -> TurnFuture<'a>;
}

struct SessionRunner(Session);

impl TurnRunner for SessionRunner {
    fn thread_id(&self) -> &str {
        &self.0.thread_id
    }

    fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    fn run_prompt<'a>(&'a self, prompt: &'a str) -> TurnFuture<'a> {
        Box::pin(self.0.ask(prompt.to_owned()))
    }
}

struct SharedState {
    status: Mutex<AutomationStatus>,
    stop_requested: AtomicBool,
    stop_notify: Notify,
}

impl SharedState {
    fn new(status: AutomationStatus) -> Self {
        Self {
            status: Mutex::new(status),
            stop_requested: AtomicBool::new(false),
            stop_notify: Notify::new(),
        }
    }

    async fn snapshot(&self) -> AutomationStatus {
        self.status.lock().await.clone()
    }
}

fn spawn_runner<R>(runner: Arc<R>, spec: AutomationSpec) -> AutomationHandle
where
    R: TurnRunner,
{
    let initial = initial_status(runner.thread_id(), runner.is_closed(), &spec);
    let shared = Arc::new(SharedState::new(initial));
    let task_shared = Arc::clone(&shared);
    let task_runner = Arc::clone(&runner);
    let join = tokio::spawn(async move { run_loop(task_runner, spec, task_shared).await });
    AutomationHandle {
        shared,
        join: Mutex::new(Some(join)),
    }
}

async fn run_loop<R>(
    runner: Arc<R>,
    spec: AutomationSpec,
    shared: Arc<SharedState>,
) -> AutomationStatus
where
    R: TurnRunner,
{
    {
        let status = shared.status.lock().await;
        if status.state.is_terminal() {
            return status.clone();
        }
    }

    let mut due_at = {
        let status = shared.status.lock().await;
        match status.next_due_at {
            Some(due_at) => due_at,
            None => {
                drop(status);
                return mark_failed(
                    &shared,
                    None,
                    "automation status invariant violated: missing next due time".to_owned(),
                )
                .await;
            }
        }
    };

    loop {
        if shared.stop_requested.load(Ordering::Acquire) {
            return mark_stopped(&shared, None).await;
        }

        if runner.is_closed() {
            return mark_failed(&shared, None, "session is closed".to_owned()).await;
        }

        if let Some(stop_at) = spec.stop_at {
            if SystemTime::now() >= stop_at || due_at >= stop_at {
                return mark_stopped(&shared, None).await;
            }
        }

        wait_until_due_or_stop(&shared, due_at).await;
        if shared.stop_requested.load(Ordering::Acquire) {
            return mark_stopped(&shared, None).await;
        }
        if runner.is_closed() {
            return mark_failed(&shared, None, "session is closed".to_owned()).await;
        }

        let started_at = SystemTime::now();
        if let Some(stop_at) = spec.stop_at {
            if started_at >= stop_at {
                return mark_stopped(&shared, None).await;
            }
        }

        {
            let mut status = shared.status.lock().await;
            status.state = AutomationState::Running;
            status.next_due_at = None;
            status.last_started_at = Some(started_at);
        }

        let result = runner.run_prompt(spec.prompt.as_str()).await;
        let finished_at = SystemTime::now();

        match result {
            Ok(_) => {
                let status = {
                    let mut status = shared.status.lock().await;
                    status.runs_completed = status.runs_completed.saturating_add(1);
                    status.last_finished_at = Some(finished_at);
                    status.last_error = None;

                    if spec
                        .max_runs
                        .is_some_and(|limit| status.runs_completed >= limit)
                    {
                        status.state = AutomationState::Stopped;
                        status.next_due_at = None;
                        return status.clone();
                    }

                    let Some(next_due) = collapse_next_due(due_at, spec.every, finished_at) else {
                        status.state = AutomationState::Failed;
                        status.next_due_at = None;
                        status.last_error =
                            Some("automation schedule overflowed next due timestamp".to_owned());
                        return status.clone();
                    };

                    if spec.stop_at.is_some_and(|stop_at| next_due >= stop_at) {
                        status.state = AutomationState::Stopped;
                        status.next_due_at = None;
                        return status.clone();
                    }

                    status.state = AutomationState::Waiting;
                    status.next_due_at = Some(next_due);
                    due_at = next_due;
                    status.clone()
                };

                if status.state.is_terminal() {
                    return status;
                }
            }
            Err(err) => {
                return mark_failed(&shared, Some(finished_at), err.to_string()).await;
            }
        }
    }
}

fn initial_status(
    thread_id: &str,
    session_closed: bool,
    spec: &AutomationSpec,
) -> AutomationStatus {
    let now = SystemTime::now();
    let mut status = AutomationStatus {
        thread_id: thread_id.to_owned(),
        runs_completed: 0,
        next_due_at: None,
        last_started_at: None,
        last_finished_at: None,
        state: AutomationState::Waiting,
        last_error: None,
    };

    if spec.every.is_zero() {
        status.state = AutomationState::Failed;
        status.last_error = Some("automation interval must be greater than zero".to_owned());
        return status;
    }

    if session_closed {
        status.state = AutomationState::Failed;
        status.last_error = Some("session is closed".to_owned());
        return status;
    }

    if spec.max_runs == Some(0) {
        status.state = AutomationState::Stopped;
        return status;
    }

    let due_at = initial_due_at(spec.start_at, now);
    if spec
        .stop_at
        .is_some_and(|stop_at| due_at >= stop_at || now >= stop_at)
    {
        status.state = AutomationState::Stopped;
        return status;
    }

    status.next_due_at = Some(due_at);
    status
}

fn initial_due_at(start_at: Option<SystemTime>, now: SystemTime) -> SystemTime {
    match start_at {
        Some(start_at) if start_at > now => start_at,
        _ => now,
    }
}

fn collapse_next_due(
    last_due_at: SystemTime,
    every: Duration,
    now: SystemTime,
) -> Option<SystemTime> {
    let next_due = checked_add_system_time(last_due_at, every)?;
    if next_due > now {
        return Some(next_due);
    }

    let overdue = now.duration_since(last_due_at).unwrap_or_default();
    let every_nanos = every.as_nanos();
    if every_nanos == 0 {
        return None;
    }
    let steps = (overdue.as_nanos() / every_nanos).saturating_add(1);
    checked_add_system_time_by_factor(last_due_at, every, steps)
}

fn checked_add_system_time(base: SystemTime, delta: Duration) -> Option<SystemTime> {
    base.checked_add(delta)
}

fn checked_add_system_time_by_factor(
    base: SystemTime,
    delta: Duration,
    factor: u128,
) -> Option<SystemTime> {
    let scaled = checked_mul_duration(delta, factor)?;
    base.checked_add(scaled)
}

fn checked_mul_duration(delta: Duration, factor: u128) -> Option<Duration> {
    let secs = (delta.as_secs() as u128).checked_mul(factor)?;
    let nanos = (delta.subsec_nanos() as u128).checked_mul(factor)?;
    let carry_secs = nanos / 1_000_000_000;
    let secs = secs.checked_add(carry_secs)?;
    let nanos = (nanos % 1_000_000_000) as u32;
    let secs = u64::try_from(secs).ok()?;
    Some(Duration::new(secs, nanos))
}

async fn wait_until_due_or_stop(shared: &SharedState, due_at: SystemTime) {
    let now = SystemTime::now();
    let Some(delay) = due_at.duration_since(now).ok() else {
        return;
    };
    tokio::select! {
        _ = tokio::time::sleep(delay) => {}
        _ = shared.stop_notify.notified() => {}
    }
}

async fn mark_stopped(shared: &SharedState, finished_at: Option<SystemTime>) -> AutomationStatus {
    let mut status = shared.status.lock().await;
    status.state = AutomationState::Stopped;
    status.next_due_at = None;
    if let Some(finished_at) = finished_at {
        status.last_finished_at = Some(finished_at);
    }
    status.clone()
}

async fn mark_failed(
    shared: &SharedState,
    finished_at: Option<SystemTime>,
    last_error: String,
) -> AutomationStatus {
    let mut status = shared.status.lock().await;
    status.state = AutomationState::Failed;
    status.next_due_at = None;
    if let Some(finished_at) = finished_at {
        status.last_finished_at = Some(finished_at);
    }
    status.last_error = Some(last_error);
    status.clone()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};

    use super::*;
    use crate::runtime::{Client, ClientConfig, SessionConfig};

    #[test]
    fn collapse_next_due_skips_missed_ticks() {
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let now = base + Duration::from_millis(35);
        let next_due = collapse_next_due(base, Duration::from_millis(10), now).expect("next due");
        assert_eq!(next_due, base + Duration::from_millis(40));
    }

    #[test]
    fn initial_status_stops_when_start_hits_stop_boundary() {
        let now = SystemTime::now();
        let at = now + Duration::from_millis(30);
        let spec = AutomationSpec {
            prompt: "night run".to_owned(),
            start_at: Some(at),
            every: Duration::from_millis(10),
            stop_at: Some(at),
            max_runs: None,
        };
        let status = initial_status("thr_test", false, &spec);
        assert_eq!(status.state, AutomationState::Stopped);
        assert_eq!(status.next_due_at, None);
    }

    #[test]
    fn initial_status_fails_when_interval_is_zero() {
        let spec = AutomationSpec {
            prompt: "night run".to_owned(),
            start_at: None,
            every: Duration::ZERO,
            stop_at: None,
            max_runs: None,
        };
        let status = initial_status("thr_test", false, &spec);
        assert_eq!(status.state, AutomationState::Failed);
        assert_eq!(
            status.last_error.as_deref(),
            Some("automation interval must be greater than zero")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runner_stop_signal_keeps_single_flight() {
        let state = Arc::new(FakeRunnerState::new(
            Duration::from_millis(40),
            vec![Ok(()), Ok(())],
        ));
        let handle = spawn_runner(
            Arc::new(FakeRunner::new("thr_stop", Arc::clone(&state))),
            AutomationSpec {
                prompt: "keep going".to_owned(),
                start_at: None,
                every: Duration::from_millis(5),
                stop_at: None,
                max_runs: None,
            },
        );

        state.first_run_started.notified().await;
        handle.stop().await;
        let status = handle.wait().await;

        assert_eq!(status.state, AutomationState::Stopped);
        assert_eq!(status.runs_completed, 1);
        assert_eq!(state.max_active.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runner_marks_failure_on_prompt_error() {
        let state = Arc::new(FakeRunnerState::new(
            Duration::ZERO,
            vec![Err(PromptRunError::TurnFailed)],
        ));

        let handle = spawn_runner(
            Arc::new(FakeRunner::new("thr_failed", Arc::clone(&state))),
            AutomationSpec {
                prompt: "fail once".to_owned(),
                start_at: None,
                every: Duration::from_millis(10),
                stop_at: None,
                max_runs: Some(3),
            },
        );

        let status = handle.wait().await;
        assert_eq!(status.state, AutomationState::Failed);
        assert_eq!(status.runs_completed, 0);
        assert_eq!(status.last_error.as_deref(), Some("turn failed"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runner_respects_delayed_start_and_max_runs() {
        let state = Arc::new(FakeRunnerState::new(Duration::ZERO, vec![Ok(()), Ok(())]));
        let start_at = SystemTime::now() + Duration::from_millis(30);

        let handle = spawn_runner(
            Arc::new(FakeRunner::new("thr_delayed", Arc::clone(&state))),
            AutomationSpec {
                prompt: "delayed".to_owned(),
                start_at: Some(start_at),
                every: Duration::from_millis(15),
                stop_at: None,
                max_runs: Some(2),
            },
        );

        let waiting = handle.status().await;
        assert_eq!(waiting.state, AutomationState::Waiting);
        assert_eq!(waiting.next_due_at, Some(start_at));

        let status = handle.wait().await;
        assert_eq!(status.state, AutomationState::Stopped);
        assert_eq!(status.runs_completed, 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runner_stops_at_stop_at_boundary_after_completed_run() {
        let state = Arc::new(FakeRunnerState::new(Duration::ZERO, vec![Ok(()), Ok(())]));
        let start_at = SystemTime::now() + Duration::from_millis(10);
        let stop_at = start_at + Duration::from_millis(20);

        let handle = spawn_runner(
            Arc::new(FakeRunner::new("thr_stop_at", Arc::clone(&state))),
            AutomationSpec {
                prompt: "bounded".to_owned(),
                start_at: Some(start_at),
                every: Duration::from_millis(20),
                stop_at: Some(stop_at),
                max_runs: None,
            },
        );

        let status = handle.wait().await;
        assert_eq!(status.state, AutomationState::Stopped);
        assert_eq!(status.runs_completed, 1);
        assert_eq!(status.next_due_at, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn automation_reuses_loaded_session_thread_for_repeated_runs() {
        let temp = TempDir::new("automation_reuse");
        let cli = write_resume_sensitive_cli_script(&temp.root, 0);
        let client = Client::connect(
            ClientConfig::new()
                .with_cli_bin(cli)
                .without_compatibility_guard(),
        )
        .await
        .expect("connect client");
        let session = client
            .start_session(SessionConfig::new(temp.root.to_string_lossy().to_string()))
            .await
            .expect("start session");
        let thread_id = session.thread_id.clone();

        let handle = spawn(
            session,
            AutomationSpec {
                prompt: "repeat".to_owned(),
                start_at: None,
                every: Duration::from_millis(10),
                stop_at: None,
                max_runs: Some(2),
            },
        );

        let status = handle.wait().await;
        assert_eq!(status.state, AutomationState::Stopped);
        assert_eq!(status.runs_completed, 2);
        assert_eq!(status.thread_id, thread_id);

        client.shutdown().await.expect("shutdown client");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn automation_fails_when_session_is_closed_before_due_run() {
        let temp = TempDir::new("automation_closed");
        let cli = write_resume_sensitive_cli_script(&temp.root, 0);
        let client = Client::connect(
            ClientConfig::new()
                .with_cli_bin(cli)
                .without_compatibility_guard(),
        )
        .await
        .expect("connect client");
        let session = client
            .start_session(SessionConfig::new(temp.root.to_string_lossy().to_string()))
            .await
            .expect("start session");
        let session_to_close = session.clone();

        let handle = spawn(
            session,
            AutomationSpec {
                prompt: "repeat".to_owned(),
                start_at: Some(SystemTime::now() + Duration::from_millis(30)),
                every: Duration::from_millis(10),
                stop_at: None,
                max_runs: Some(2),
            },
        );

        session_to_close.close().await.expect("close session");
        let status = handle.wait().await;

        assert_eq!(status.state, AutomationState::Failed);
        assert_eq!(status.runs_completed, 0);
        assert!(status
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("session is closed")));

        client.shutdown().await.expect("shutdown client");
    }

    struct FakeRunner {
        thread_id: String,
        state: Arc<FakeRunnerState>,
    }

    impl FakeRunner {
        fn new(thread_id: &str, state: Arc<FakeRunnerState>) -> Self {
            Self {
                thread_id: thread_id.to_owned(),
                state,
            }
        }
    }

    impl TurnRunner for FakeRunner {
        fn thread_id(&self) -> &str {
            &self.thread_id
        }

        fn is_closed(&self) -> bool {
            false
        }

        fn run_prompt<'a>(&'a self, _prompt: &'a str) -> TurnFuture<'a> {
            Box::pin(async move {
                let active = self.state.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.state.max_active.fetch_max(active, Ordering::SeqCst);
                self.state.first_run_started.notify_waiters();
                if !self.state.delay.is_zero() {
                    tokio::time::sleep(self.state.delay).await;
                }
                self.state.active.fetch_sub(1, Ordering::SeqCst);
                let next = self
                    .state
                    .results
                    .lock()
                    .expect("results lock")
                    .pop_front()
                    .unwrap_or(Ok(()));
                next.map(|_| PromptRunResult {
                    thread_id: self.thread_id.clone(),
                    turn_id: format!(
                        "turn_{}",
                        self.state.turn_counter.fetch_add(1, Ordering::SeqCst)
                    ),
                    assistant_text: "ok".to_owned(),
                })
            })
        }
    }

    struct FakeRunnerState {
        delay: Duration,
        results: StdMutex<VecDeque<Result<(), PromptRunError>>>,
        active: AtomicUsize,
        max_active: AtomicUsize,
        turn_counter: AtomicUsize,
        first_run_started: Notify,
    }

    impl FakeRunnerState {
        fn new(delay: Duration, results: Vec<Result<(), PromptRunError>>) -> Self {
            Self {
                delay,
                results: StdMutex::new(VecDeque::from(results)),
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
                turn_counter: AtomicUsize::new(0),
                first_run_started: Notify::new(),
            }
        }
    }

    #[derive(Debug)]
    struct TempDir {
        root: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let root = std::env::temp_dir().join(format!("{prefix}_{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&root).expect("create temp root");
            Self { root }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn write_resume_sensitive_cli_script(root: &Path, allowed_resume_calls: usize) -> PathBuf {
        let path = root.join("mock_codex_cli_resume_sensitive.py");
        let script = r#"#!/usr/bin/env python3
import json
import sys

allowed_resume_calls = __ALLOWED_RESUME_CALLS__
resume_calls = 0

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception:
        continue

    rpc_id = msg.get("id")
    method = msg.get("method")
    params = msg.get("params") or {}

    if rpc_id is None:
        continue

    if method == "initialize":
        sys.stdout.write(json.dumps({
            "id": rpc_id,
            "result": {"ready": True, "userAgent": "Codex Desktop/0.104.0"}
        }) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/start":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": "thr_automation"}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/resume":
        resume_calls += 1
        if resume_calls > allowed_resume_calls:
            sys.stdout.write(json.dumps({
                "id": rpc_id,
                "error": {"code": -32002, "message": f"unexpected thread/resume call #{resume_calls}"}
            }) + "\n")
        else:
            thread_id = params.get("threadId") or "thr_automation"
            sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        thread_id = params.get("threadId") or "thr_automation"
        turn_id = "turn_" + str(resume_calls)
        text = "ok"
        input_items = params.get("input") or []
        if input_items and isinstance(input_items[0], dict):
            text = input_items[0].get("text") or "ok"

        sys.stdout.write(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/started","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_1","itemType":"agentMessage"}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/agentMessage/delta","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_1","delta":text}}) + "\n")
        sys.stdout.write(json.dumps({"method":"turn/completed","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"turn": {"id": turn_id}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/archive":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ok": True}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ok": True}}) + "\n")
    sys.stdout.flush()
"#;
        let script = script.replace(
            "__ALLOWED_RESUME_CALLS__",
            &allowed_resume_calls.to_string(),
        );
        fs::write(&path, script).expect("write cli script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(&path).expect("metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).expect("set executable");
        }
        path
    }
}

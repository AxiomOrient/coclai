use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub(crate) struct AgentRuntimePaths {
    pub(crate) state_dir: PathBuf,
    pub(crate) lock_path: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct ProcessState {
    pub(crate) running: bool,
    pub(crate) pid: Option<u32>,
    pub(crate) stale_lock: bool,
    pub(crate) lock_path: PathBuf,
}

pub(crate) struct AgentLockGuard {
    lock_path: PathBuf,
}

impl Drop for AgentLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

pub(crate) fn runtime_paths() -> AgentRuntimePaths {
    if let Ok(dir) = std::env::var("COCLAI_AGENT_STATE_DIR") {
        let path = PathBuf::from(dir);
        return AgentRuntimePaths {
            lock_path: path.join("agent.lock"),
            state_dir: path,
        };
    }

    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let state_dir = base.join(".coclai").join("agent");
    AgentRuntimePaths {
        lock_path: state_dir.join("agent.lock"),
        state_dir,
    }
}

fn ensure_state_dir(paths: &AgentRuntimePaths) -> Result<(), String> {
    fs::create_dir_all(&paths.state_dir).map_err(|err| {
        format!(
            "failed to create state dir {}: {err}",
            paths.state_dir.display()
        )
    })
}

pub(crate) fn parse_lock_pid(content: &str) -> Option<u32> {
    content.lines().find_map(|line| {
        let value = line.strip_prefix("pid=")?;
        value.trim().parse::<u32>().ok()
    })
}

fn pid_is_running(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn signal_pid(pid: u32, signal: &str) -> Result<(), String> {
    let status = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .status()
        .map_err(|err| format!("failed to run kill -{signal} {pid}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("kill -{signal} {pid} failed with status {status}"))
    }
}

pub(crate) fn inspect_process_state(paths: &AgentRuntimePaths) -> ProcessState {
    if !paths.lock_path.exists() {
        return ProcessState {
            running: false,
            pid: None,
            stale_lock: false,
            lock_path: paths.lock_path.clone(),
        };
    }

    let content = match fs::read_to_string(&paths.lock_path) {
        Ok(content) => content,
        Err(_) => {
            return ProcessState {
                running: false,
                pid: None,
                stale_lock: true,
                lock_path: paths.lock_path.clone(),
            }
        }
    };

    let pid = parse_lock_pid(&content);
    if let Some(pid) = pid {
        if pid_is_running(pid) {
            return ProcessState {
                running: true,
                pid: Some(pid),
                stale_lock: false,
                lock_path: paths.lock_path.clone(),
            };
        }
    }

    ProcessState {
        running: false,
        pid,
        stale_lock: true,
        lock_path: paths.lock_path.clone(),
    }
}

pub(crate) fn acquire_single_instance_lock(
    paths: &AgentRuntimePaths,
) -> Result<AgentLockGuard, String> {
    ensure_state_dir(paths)?;
    let state = inspect_process_state(paths);
    if state.running {
        return Err(format!(
            "agent already running (pid={})",
            state
                .pid
                .map_or_else(|| "unknown".to_owned(), |pid| pid.to_string())
        ));
    }
    if state.stale_lock && paths.lock_path.exists() {
        fs::remove_file(&paths.lock_path).map_err(|err| {
            format!(
                "failed to remove stale lock {}: {err}",
                paths.lock_path.display()
            )
        })?;
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("system time error: {err}"))?
        .as_secs();
    let pid = std::process::id();
    let mut lock_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&paths.lock_path)
        .map_err(|err| {
            format!(
                "failed to create single-instance lock {}: {err}",
                paths.lock_path.display()
            )
        })?;
    writeln!(lock_file, "pid={pid}")
        .and_then(|_| writeln!(lock_file, "started_unix={now}"))
        .map_err(|err| {
            format!(
                "failed to write lock metadata {}: {err}",
                paths.lock_path.display()
            )
        })?;

    Ok(AgentLockGuard {
        lock_path: paths.lock_path.clone(),
    })
}

pub(crate) fn start_background(
    paths: &AgentRuntimePaths,
    bind_addr: SocketAddr,
) -> Result<Value, String> {
    ensure_state_dir(paths)?;

    let current = inspect_process_state(paths);
    if current.running {
        return Ok(json!({
            "status": "already_running",
            "pid": current.pid,
            "lock_path": paths.lock_path.display().to_string(),
        }));
    }
    if current.stale_lock && paths.lock_path.exists() {
        let _ = fs::remove_file(&paths.lock_path);
    }

    let exe = std::env::current_exe().map_err(|err| format!("current_exe failed: {err}"))?;
    let mut child = Command::new(exe)
        .arg("serve")
        .arg("--bind")
        .arg(bind_addr.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("failed to spawn background agent: {err}"))?;
    let child_pid = child.id();

    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Some(status) = child
            .try_wait()
            .map_err(|err| format!("failed to poll spawned process status: {err}"))?
        {
            return Ok(json!({
                "status": "start_failed",
                "spawned_pid": child_pid,
                "exit_status": status.code(),
                "lock_path": paths.lock_path.display().to_string(),
            }));
        }

        let state = inspect_process_state(paths);
        if state.running {
            return Ok(json!({
                "status": "started",
                "pid": state.pid,
                "lock_path": paths.lock_path.display().to_string(),
            }));
        }
        thread::sleep(Duration::from_millis(100));
    }

    Ok(json!({
        "status": "start_timeout",
        "spawned_pid": child_pid,
        "lock_path": paths.lock_path.display().to_string(),
    }))
}

pub(crate) fn stop_background(paths: &AgentRuntimePaths) -> Result<Value, String> {
    let state = inspect_process_state(paths);
    if !state.running {
        if state.stale_lock && paths.lock_path.exists() {
            let _ = fs::remove_file(&paths.lock_path);
        }
        return Ok(json!({
            "status": "not_running",
            "lock_path": paths.lock_path.display().to_string(),
            "stale_lock_removed": state.stale_lock,
        }));
    }

    let pid = state
        .pid
        .ok_or_else(|| "running state without pid".to_owned())?;
    signal_pid(pid, "TERM")?;

    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !pid_is_running(pid) {
            let _ = fs::remove_file(&paths.lock_path);
            return Ok(json!({
                "status": "stopped",
                "pid": pid,
            }));
        }
        thread::sleep(Duration::from_millis(100));
    }

    signal_pid(pid, "KILL")?;
    let _ = fs::remove_file(&paths.lock_path);
    Ok(json!({
        "status": "killed",
        "pid": pid,
    }))
}

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use super::lock_policy::{parse_lock_metadata, should_reap_lock, LockMetadata, LockOwnerStatus};
use super::models::compute_revision;
use super::{ArtifactMeta, ArtifactStore, FsArtifactStore, SaveMeta, StoreErr};

const LOCK_STALE_FALLBACK_AGE: Duration = Duration::from_secs(30);

impl FsArtifactStore {
    const LOCK_WAIT_TIMEOUT: Duration = Duration::from_secs(2);
    const LOCK_RETRY_DELAY: Duration = Duration::from_millis(5);

    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn artifact_dir(&self, artifact_id: &str) -> std::path::PathBuf {
        self.root.join(artifact_key(artifact_id))
    }

    fn text_path(&self, artifact_id: &str) -> std::path::PathBuf {
        self.artifact_dir(artifact_id).join("text.txt")
    }

    fn meta_path(&self, artifact_id: &str) -> std::path::PathBuf {
        self.artifact_dir(artifact_id).join("meta.json")
    }

    fn save_meta_path(&self, artifact_id: &str) -> std::path::PathBuf {
        self.artifact_dir(artifact_id).join("last_save_meta.json")
    }

    fn lock_path(&self, artifact_id: &str) -> std::path::PathBuf {
        self.artifact_dir(artifact_id).join(".artifact.lock")
    }

    fn ensure_artifact_dir(&self, artifact_id: &str) -> Result<(), StoreErr> {
        let dir = self.artifact_dir(artifact_id);
        fs::create_dir_all(&dir)
            .map_err(|err| StoreErr::Io(format!("create artifact dir failed: {err}")))
    }

    fn load_current_revision(&self, artifact_id: &str) -> Result<String, StoreErr> {
        let text_path = self.text_path(artifact_id);
        let current_text = read_optional_existing_text(&text_path)?;
        Ok(compute_revision(&current_text))
    }

    fn with_artifact_lock<T>(
        &self,
        artifact_id: &str,
        f: impl FnOnce() -> Result<T, StoreErr>,
    ) -> Result<T, StoreErr> {
        let lock = self.acquire_lock(artifact_id)?;
        let result = f();
        drop(lock);
        result
    }

    fn acquire_lock(&self, artifact_id: &str) -> Result<ArtifactLock, StoreErr> {
        let lock_path = self.lock_path(artifact_id);
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| StoreErr::Io(format!("create lock dir failed: {err}")))?;
        }

        let started = Instant::now();
        loop {
            match fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&lock_path)
            {
                Ok(mut file) => {
                    write_lock_metadata(&mut file)?;
                    return Ok(ArtifactLock {
                        path: lock_path,
                        file,
                    });
                }
                Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                    if lock_owner_is_stale(&lock_path) {
                        match fs::remove_file(&lock_path) {
                            Ok(()) => continue,
                            Err(remove_err) if remove_err.kind() == ErrorKind::NotFound => {
                                continue;
                            }
                            Err(_) => {}
                        }
                    }
                    if started.elapsed() >= Self::LOCK_WAIT_TIMEOUT {
                        return Err(StoreErr::Io(format!(
                            "artifact lock timed out: {}",
                            lock_path.to_string_lossy()
                        )));
                    }
                    thread::sleep(Self::LOCK_RETRY_DELAY);
                }
                Err(err) => {
                    return Err(StoreErr::Io(format!(
                        "artifact lock failed at {}: {err}",
                        lock_path.to_string_lossy()
                    )))
                }
            }
        }
    }
}

fn write_lock_metadata(file: &mut fs::File) -> Result<(), StoreErr> {
    let pid = std::process::id();
    let created_unix_ms = now_unix_millis();
    let payload = format!("{pid}:{created_unix_ms}\n");
    file.write_all(payload.as_bytes())
        .map_err(|err| StoreErr::Io(format!("write lock metadata failed: {err}")))?;
    file.sync_all()
        .map_err(|err| StoreErr::Io(format!("sync lock metadata failed: {err}")))?;
    Ok(())
}

fn lock_owner_is_stale(path: &Path) -> bool {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => return false,
    };
    let metadata = match parse_lock_metadata(&raw) {
        Some(metadata) => metadata,
        None => return false,
    };

    let now_unix_ms = now_unix_millis();
    let created_unix_ms = resolve_lock_created_unix_millis(path, &metadata);
    let owner_status = match process_is_alive(metadata.pid) {
        Some(true) => LockOwnerStatus::Alive,
        Some(false) => LockOwnerStatus::Dead,
        None => LockOwnerStatus::Unknown,
    };

    should_reap_lock(
        owner_status,
        created_unix_ms,
        now_unix_ms,
        LOCK_STALE_FALLBACK_AGE,
    )
}

fn resolve_lock_created_unix_millis(path: &Path, metadata: &LockMetadata) -> Option<u64> {
    if metadata.created_unix_ms > 0 {
        return Some(metadata.created_unix_ms);
    }

    fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> Option<bool> {
    let status = Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(status) => Some(status.success()),
        Err(_) => None,
    }
}

#[cfg(not(unix))]
fn process_is_alive(_pid: u32) -> Option<bool> {
    None
}

fn now_unix_millis() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as u64,
        Err(_) => 0,
    }
}

impl ArtifactStore for FsArtifactStore {
    fn load_text(&self, artifact_id: &str) -> Result<String, StoreErr> {
        let path = self.text_path(artifact_id);
        read_to_string_checked(&path, artifact_id)
    }

    fn save_text(&self, artifact_id: &str, new_text: &str, meta: SaveMeta) -> Result<(), StoreErr> {
        self.with_artifact_lock(artifact_id, || {
            self.ensure_artifact_dir(artifact_id)?;

            let actual_revision = self.load_current_revision(artifact_id)?;
            let text_path = self.text_path(artifact_id);
            if let Some(expected_revision) = meta.previous_revision.as_deref() {
                if expected_revision != actual_revision {
                    return Err(StoreErr::Conflict {
                        expected: expected_revision.to_owned(),
                        actual: actual_revision,
                    });
                }
            }

            let payload = serde_json::to_vec(&meta)
                .map_err(|err| StoreErr::Serialize(format!("serialize save meta failed: {err}")))?;
            write_atomic_bytes(&self.save_meta_path(artifact_id), &payload)?;
            // Commit ordering: save metadata first, then text.
            // This avoids returning an error after text has already been committed.
            write_atomic_text(&text_path, new_text)?;

            Ok(())
        })
    }

    fn save_text_and_meta(
        &self,
        artifact_id: &str,
        new_text: &str,
        save_meta: SaveMeta,
        meta: ArtifactMeta,
    ) -> Result<(), StoreErr> {
        self.with_artifact_lock(artifact_id, || {
            self.ensure_artifact_dir(artifact_id)?;

            let actual_revision = self.load_current_revision(artifact_id)?;
            if let Some(expected_revision) = save_meta.previous_revision.as_deref() {
                if expected_revision != actual_revision {
                    return Err(StoreErr::Conflict {
                        expected: expected_revision.to_owned(),
                        actual: actual_revision,
                    });
                }
            }

            let computed_next_revision = compute_revision(new_text);
            if save_meta.next_revision != computed_next_revision {
                return Err(StoreErr::Conflict {
                    expected: save_meta.next_revision.clone(),
                    actual: computed_next_revision,
                });
            }
            if meta.revision != save_meta.next_revision {
                return Err(StoreErr::Conflict {
                    expected: save_meta.next_revision.clone(),
                    actual: meta.revision.clone(),
                });
            }

            let text_path = self.text_path(artifact_id);
            let old_text = read_optional_existing_text(&text_path)?;
            let save_meta_bytes = serde_json::to_vec(&save_meta)
                .map_err(|err| StoreErr::Serialize(format!("serialize save meta failed: {err}")))?;
            let meta_bytes = serde_json::to_vec(&meta).map_err(|err| {
                StoreErr::Serialize(format!("serialize artifact meta failed: {err}"))
            })?;

            write_atomic_bytes(&self.save_meta_path(artifact_id), &save_meta_bytes)?;
            write_atomic_text(&text_path, new_text)?;
            if let Err(meta_err) = write_atomic_bytes(&self.meta_path(artifact_id), &meta_bytes) {
                let rollback = write_atomic_text(&text_path, &old_text);
                if let Err(rollback_err) = rollback {
                    return Err(StoreErr::Io(format!(
                        "persist artifact meta failed after text commit: {meta_err}; text rollback failed: {rollback_err}"
                    )));
                }
                return Err(meta_err);
            }

            Ok(())
        })
    }

    fn get_meta(&self, artifact_id: &str) -> Result<ArtifactMeta, StoreErr> {
        let path = self.meta_path(artifact_id);
        let bytes = read_checked(&path, artifact_id)?;
        serde_json::from_slice::<ArtifactMeta>(&bytes)
            .map_err(|err| StoreErr::Serialize(format!("parse artifact meta failed: {err}")))
    }

    fn set_meta(&self, artifact_id: &str, meta: ArtifactMeta) -> Result<(), StoreErr> {
        self.with_artifact_lock(artifact_id, || {
            self.ensure_artifact_dir(artifact_id)?;

            let actual_revision = self.load_current_revision(artifact_id)?;
            if meta.revision != actual_revision {
                return Err(StoreErr::Conflict {
                    expected: meta.revision,
                    actual: actual_revision,
                });
            }

            let bytes = serde_json::to_vec(&meta).map_err(|err| {
                StoreErr::Serialize(format!("serialize artifact meta failed: {err}"))
            })?;
            write_atomic_bytes(&self.meta_path(artifact_id), &bytes)?;

            Ok(())
        })
    }
}

/// Read existing text or return empty string if not found.
/// Allocation: one String (file contents). Complexity: O(n), n=file size.
fn read_optional_existing_text(path: &Path) -> Result<String, StoreErr> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(StoreErr::Io(format!(
            "read current artifact text failed: {err}"
        ))),
    }
}

fn read_to_string_checked(path: &Path, artifact_id: &str) -> Result<String, StoreErr> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            Err(StoreErr::NotFound(artifact_id.to_owned()))
        }
        Err(err) => Err(StoreErr::Io(format!("read text failed: {err}"))),
    }
}

fn read_checked(path: &Path, artifact_id: &str) -> Result<Vec<u8>, StoreErr> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            Err(StoreErr::NotFound(artifact_id.to_owned()))
        }
        Err(err) => Err(StoreErr::Io(format!("read file failed: {err}"))),
    }
}

/// Stable artifact path key: visible prefix + hash suffix.
/// Allocation: one String. Complexity: O(n), n=artifact_id length.
pub(crate) fn artifact_key(artifact_id: &str) -> String {
    let mut prefix = String::with_capacity(artifact_id.len());
    for ch in artifact_id.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            prefix.push(ch);
        } else {
            prefix.push('_');
        }
    }
    if prefix.is_empty() {
        prefix.push_str("artifact");
    }

    let mut hasher = Sha256::new();
    hasher.update(artifact_id.as_bytes());
    let digest = hex::encode(hasher.finalize());
    let short = &digest[..12];
    format!("{prefix}_{short}")
}

fn write_atomic_text(path: &Path, text: &str) -> Result<(), StoreErr> {
    write_atomic_bytes(path, text.as_bytes())
}

fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> Result<(), StoreErr> {
    let temp_path = temp_path_for(path);
    fs::write(&temp_path, bytes).map_err(|err| {
        StoreErr::Io(format!(
            "write temp file failed at {}: {err}",
            temp_path.to_string_lossy()
        ))
    })?;
    if let Err(err) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(StoreErr::Io(format!(
            "atomic rename failed {} -> {}: {err}",
            temp_path.to_string_lossy(),
            path.to_string_lossy()
        )));
    }
    Ok(())
}

fn temp_path_for(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("tmp");
    path.with_file_name(format!("{name}.tmp-{}", std::process::id()))
}

struct ArtifactLock {
    path: PathBuf,
    file: fs::File,
}

impl Drop for ArtifactLock {
    fn drop(&mut self) {
        let _ = self.file.sync_all();
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(unix))]
    #[test]
    fn non_unix_pid_probe_falls_back_to_unknown_owner_status() {
        assert_eq!(super::process_is_alive(123), None);
        assert!(super::should_reap_lock(
            super::LockOwnerStatus::Unknown,
            Some(0),
            super::LOCK_STALE_FALLBACK_AGE.as_millis() as u64 + 1,
            super::LOCK_STALE_FALLBACK_AGE,
        ));
    }

    #[cfg(unix)]
    #[test]
    fn unix_pid_probe_returns_dead_for_nonexistent_process() {
        // u32::MAX is not a valid PID on any unix system; kill -0 exits non-zero.
        assert_eq!(super::process_is_alive(u32::MAX), Some(false));
        // Dead owner status => reap immediately regardless of age.
        assert!(super::should_reap_lock(
            super::LockOwnerStatus::Dead,
            Some(u64::MAX),
            0,
            super::LOCK_STALE_FALLBACK_AGE,
        ));
    }
}

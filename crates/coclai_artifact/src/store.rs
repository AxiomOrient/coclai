use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use super::{ArtifactMeta, ArtifactStore, FsArtifactStore, SaveMeta, StoreErr};

impl FsArtifactStore {
    const LOCK_WAIT_TIMEOUT: Duration = Duration::from_secs(2);
    const LOCK_RETRY_DELAY: Duration = Duration::from_millis(5);
    const LOCK_STALE_AFTER: Duration = Duration::from_secs(30);

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
                    if lock_is_stale(&lock_path) {
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

fn lock_is_stale(path: &Path) -> bool {
    let now = now_unix_millis();
    let stale_window_ms = FsArtifactStore::LOCK_STALE_AFTER.as_millis() as u64;

    if let Ok(raw) = fs::read_to_string(path) {
        if let Some(created_unix_ms) = parse_lock_created_unix_ms(&raw) {
            if now.saturating_sub(created_unix_ms) >= stale_window_ms {
                return true;
            }
        }
    }

    if let Ok(metadata) = fs::metadata(path) {
        if let Ok(modified_at) = metadata.modified() {
            if let Ok(elapsed) = modified_at.elapsed() {
                return elapsed >= FsArtifactStore::LOCK_STALE_AFTER;
            }
        }
    }

    false
}

fn parse_lock_created_unix_ms(raw: &str) -> Option<u64> {
    let (_, ts) = raw.trim().split_once(':')?;
    ts.parse::<u64>().ok()
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
            let dir = self.artifact_dir(artifact_id);
            fs::create_dir_all(&dir)
                .map_err(|err| StoreErr::Io(format!("create artifact dir failed: {err}")))?;

            let text_path = self.text_path(artifact_id);
            let current_text = match fs::read_to_string(&text_path) {
                Ok(text) => text,
                Err(err) if err.kind() == ErrorKind::NotFound => String::new(),
                Err(err) => {
                    return Err(StoreErr::Io(format!(
                        "read current artifact text failed: {err}"
                    )))
                }
            };
            let actual_revision = compute_text_revision(&current_text);
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

    fn get_meta(&self, artifact_id: &str) -> Result<ArtifactMeta, StoreErr> {
        let path = self.meta_path(artifact_id);
        let bytes = read_checked(&path, artifact_id)?;
        serde_json::from_slice::<ArtifactMeta>(&bytes)
            .map_err(|err| StoreErr::Serialize(format!("parse artifact meta failed: {err}")))
    }

    fn set_meta(&self, artifact_id: &str, meta: ArtifactMeta) -> Result<(), StoreErr> {
        self.with_artifact_lock(artifact_id, || {
            let dir = self.artifact_dir(artifact_id);
            fs::create_dir_all(&dir)
                .map_err(|err| StoreErr::Io(format!("create artifact dir failed: {err}")))?;

            let text_path = self.text_path(artifact_id);
            let current_text = match fs::read_to_string(&text_path) {
                Ok(text) => text,
                Err(err) if err.kind() == ErrorKind::NotFound => String::new(),
                Err(err) => {
                    return Err(StoreErr::Io(format!(
                        "read current artifact text failed: {err}"
                    )))
                }
            };
            let actual_revision = compute_text_revision(&current_text);
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

fn read_to_string_checked(path: &Path, artifact_id: &str) -> Result<String, StoreErr> {
    if !path.exists() {
        return Err(StoreErr::NotFound(artifact_id.to_owned()));
    }
    fs::read_to_string(path).map_err(|err| StoreErr::Io(format!("read text failed: {err}")))
}

fn read_checked(path: &Path, artifact_id: &str) -> Result<Vec<u8>, StoreErr> {
    if !path.exists() {
        return Err(StoreErr::NotFound(artifact_id.to_owned()));
    }
    fs::read(path).map_err(|err| StoreErr::Io(format!("read file failed: {err}")))
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

fn compute_text_revision(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
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

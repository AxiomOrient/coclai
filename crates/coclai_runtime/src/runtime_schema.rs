use std::fs;
use std::path::Path;

use crate::errors::RuntimeError;
use crate::runtime::SchemaGuardConfig;
use crate::schema::{validate_metadata_fields, validate_schema_manifest, ManifestFile};
use crate::state::StateProjectionLimits;

/// Validate schema metadata+manifest before runtime startup.
/// Side effects: filesystem reads only. Complexity: O(n log n), n = schema file count.
pub(crate) fn validate_schema_guard(cfg: &SchemaGuardConfig) -> Result<(), RuntimeError> {
    let metadata_path = cfg.active_schema_dir.join("metadata.json");
    let manifest_path = cfg.active_schema_dir.join("manifest.sha256");
    let schema_dir = cfg.active_schema_dir.join("json-schema");

    let metadata_contents = fs::read_to_string(&metadata_path).map_err(|err| {
        RuntimeError::Internal(format!(
            "failed to read metadata.json at {:?}: {err}",
            metadata_path
        ))
    })?;
    validate_metadata_fields(&metadata_contents).map_err(|err| {
        RuntimeError::Internal(format!(
            "invalid schema metadata at {:?}: {err}",
            metadata_path
        ))
    })?;

    let manifest_contents = fs::read_to_string(&manifest_path).map_err(|err| {
        RuntimeError::Internal(format!(
            "failed to read manifest.sha256 at {:?}: {err}",
            manifest_path
        ))
    })?;

    let files = load_schema_files(&schema_dir)?;
    validate_schema_manifest(&manifest_contents, &files).map_err(|err| {
        RuntimeError::Internal(format!(
            "schema manifest validation failed at {:?}: {err}",
            manifest_path
        ))
    })?;

    Ok(())
}

pub(crate) fn validate_runtime_capacities(
    live_channel_capacity: usize,
    server_request_channel_capacity: usize,
    has_event_sink: bool,
    event_sink_channel_capacity: usize,
    rpc_response_timeout: std::time::Duration,
) -> Result<(), RuntimeError> {
    if live_channel_capacity == 0 {
        return Err(RuntimeError::InvalidConfig(
            "live_channel_capacity must be > 0".to_owned(),
        ));
    }
    if server_request_channel_capacity == 0 {
        return Err(RuntimeError::InvalidConfig(
            "server_request_channel_capacity must be > 0".to_owned(),
        ));
    }
    if has_event_sink && event_sink_channel_capacity == 0 {
        return Err(RuntimeError::InvalidConfig(
            "event_sink_channel_capacity must be > 0 when event_sink is configured".to_owned(),
        ));
    }
    if rpc_response_timeout.is_zero() {
        return Err(RuntimeError::InvalidConfig(
            "rpc_response_timeout must be > 0".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_state_projection_limits(
    limits: &StateProjectionLimits,
) -> Result<(), RuntimeError> {
    if limits.max_threads == 0 {
        return Err(RuntimeError::InvalidConfig(
            "state_projection_limits.max_threads must be > 0".to_owned(),
        ));
    }
    if limits.max_turns_per_thread == 0 {
        return Err(RuntimeError::InvalidConfig(
            "state_projection_limits.max_turns_per_thread must be > 0".to_owned(),
        ));
    }
    if limits.max_items_per_turn == 0 {
        return Err(RuntimeError::InvalidConfig(
            "state_projection_limits.max_items_per_turn must be > 0".to_owned(),
        ));
    }
    if limits.max_text_bytes_per_item == 0 {
        return Err(RuntimeError::InvalidConfig(
            "state_projection_limits.max_text_bytes_per_item must be > 0".to_owned(),
        ));
    }
    if limits.max_stdout_bytes_per_item == 0 {
        return Err(RuntimeError::InvalidConfig(
            "state_projection_limits.max_stdout_bytes_per_item must be > 0".to_owned(),
        ));
    }
    if limits.max_stderr_bytes_per_item == 0 {
        return Err(RuntimeError::InvalidConfig(
            "state_projection_limits.max_stderr_bytes_per_item must be > 0".to_owned(),
        ));
    }
    Ok(())
}

fn load_schema_files(schema_dir: &Path) -> Result<Vec<ManifestFile>, RuntimeError> {
    if !schema_dir.exists() {
        return Err(RuntimeError::Internal(format!(
            "schema directory not found: {:?}",
            schema_dir
        )));
    }

    let mut files = Vec::new();
    collect_schema_files(schema_dir, schema_dir, &mut files)?;
    Ok(files)
}

fn collect_schema_files(
    root: &Path,
    current: &Path,
    out: &mut Vec<ManifestFile>,
) -> Result<(), RuntimeError> {
    let entries = fs::read_dir(current).map_err(|err| {
        RuntimeError::Internal(format!("failed to read schema dir {:?}: {err}", current))
    })?;

    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| {
            RuntimeError::Internal(format!(
                "failed to read schema entry in {:?}: {err}",
                current
            ))
        })?;
        paths.push(entry.path());
    }
    paths.sort();

    for path in paths {
        if path.is_dir() {
            collect_schema_files(root, &path, out)?;
            continue;
        }
        if !path.is_file() {
            continue;
        }

        let rel = path.strip_prefix(root).map_err(|err| {
            RuntimeError::Internal(format!(
                "failed to strip schema root prefix for {:?} in {:?}: {err}",
                path, root
            ))
        })?;
        let rel = rel.to_string_lossy().replace('\\', "/");
        let bytes = fs::read(&path).map_err(|err| {
            RuntimeError::Internal(format!("failed to read schema file {:?}: {err}", path))
        })?;
        out.push(ManifestFile {
            relative_path: format!("./{rel}"),
            bytes,
        });
    }

    Ok(())
}

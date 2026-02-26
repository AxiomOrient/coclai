use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestFile {
    pub relative_path: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataFields {
    pub schema_name: String,
    pub generated_at_utc: String,
    pub generator_command: String,
    pub source_of_truth: String,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ManifestMismatch {
    #[error("manifest mismatch")]
    Mismatch,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum MetadataValidationError {
    #[error("metadata is not valid json: {0}")]
    InvalidJson(String),
    #[error("metadata field is missing: {0}")]
    MissingField(&'static str),
    #[error("metadata field is empty: {0}")]
    EmptyField(&'static str),
}

/// Validate required metadata fields and return the normalized values.
/// Allocation: up to 4 owned Strings. Complexity: O(1).
pub fn validate_metadata_fields(
    metadata_contents: &str,
) -> Result<MetadataFields, MetadataValidationError> {
    let json: Value = serde_json::from_str(metadata_contents)
        .map_err(|err| MetadataValidationError::InvalidJson(err.to_string()))?;

    let schema_name = required_non_empty_string(&json, "schemaName")?;
    let generated_at_utc = required_non_empty_string(&json, "generatedAtUtc")?;
    let generator_command = required_non_empty_string(&json, "generatorCommand")?;
    let source_of_truth = required_non_empty_string(&json, "sourceOfTruth")?;

    Ok(MetadataFields {
        schema_name,
        generated_at_utc,
        generator_command,
        source_of_truth,
    })
}

/// Validate manifest text against in-memory files.
/// Allocation: one (path,digest) vec + rendered manifest string.
/// Complexity: O(n log n) for path sort, n = file count.
pub fn validate_schema_manifest(
    manifest_contents: &str,
    files: &[ManifestFile],
) -> Result<(), ManifestMismatch> {
    let mut hashed: Vec<(String, String)> = files
        .iter()
        .map(|f| {
            let mut hasher = Sha256::new();
            hasher.update(&f.bytes);
            let digest = hex::encode(hasher.finalize());
            (f.relative_path.clone(), digest)
        })
        .collect();

    hashed.sort_by(|a, b| a.0.cmp(&b.0));

    let actual = hashed
        .iter()
        .map(|(path, digest)| format!("{digest}  {path}"))
        .collect::<Vec<_>>()
        .join("\n");

    if normalize_newline(manifest_contents) == normalize_newline(&actual) {
        return Ok(());
    }

    Err(ManifestMismatch::Mismatch)
}

fn normalize_newline(s: &str) -> String {
    s.trim().replace("\r\n", "\n")
}

fn required_non_empty_string(
    json: &Value,
    key: &'static str,
) -> Result<String, MetadataValidationError> {
    let Some(value) = json.get(key) else {
        return Err(MetadataValidationError::MissingField(key));
    };
    let Some(value) = value.as_str() else {
        return Err(MetadataValidationError::MissingField(key));
    };
    if value.trim().is_empty() {
        return Err(MetadataValidationError::EmptyField(key));
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_metadata_ok() {
        let metadata = r#"{
  "schemaName":"app-server",
  "generatedAtUtc":"2026-01-01T00:00:00Z",
  "generatorCommand":"codex app-server generate-json-schema --out <DIR>",
  "sourceOfTruth":"active/json-schema"
}"#;

        let fields = validate_metadata_fields(metadata).expect("metadata should parse");
        assert_eq!(fields.schema_name, "app-server");
        assert_eq!(fields.source_of_truth, "active/json-schema");
    }

    #[test]
    fn validate_metadata_missing_field() {
        let metadata = r#"{"schemaName":"app-server"}"#;
        let err = validate_metadata_fields(metadata).expect_err("metadata should fail");
        assert_eq!(err, MetadataValidationError::MissingField("generatedAtUtc"));
    }

    #[test]
    fn validate_metadata_empty_field() {
        let metadata = r#"{
  "schemaName":"app-server",
  "generatedAtUtc":" ",
  "generatorCommand":"x",
  "sourceOfTruth":"y"
}"#;
        let err = validate_metadata_fields(metadata).expect_err("metadata should fail");
        assert_eq!(err, MetadataValidationError::EmptyField("generatedAtUtc"));
    }

    #[test]
    fn validate_manifest_ok() {
        let files = vec![ManifestFile {
            relative_path: "./schema.json".to_owned(),
            bytes: br#"{"type":"object"}"#.to_vec(),
        }];

        let mut hasher = Sha256::new();
        hasher.update(br#"{"type":"object"}"#);
        let expected = format!("{}  ./schema.json", hex::encode(hasher.finalize()));

        assert_eq!(validate_schema_manifest(&expected, &files), Ok(()));
    }

    #[test]
    fn validate_manifest_err() {
        let files = vec![ManifestFile {
            relative_path: "./schema.json".to_owned(),
            bytes: b"{}".to_vec(),
        }];

        assert_eq!(
            validate_schema_manifest("deadbeef  ./schema.json", &files),
            Err(ManifestMismatch::Mismatch)
        );
    }
}

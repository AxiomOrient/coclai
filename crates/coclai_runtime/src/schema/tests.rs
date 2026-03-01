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

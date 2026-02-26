use std::path::Path;

use coclai_runtime::{classify_message, extract_ids, MsgKind};

fn load_json(path: &str) -> serde_json::Value {
    let full = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    let raw = std::fs::read_to_string(full).expect("fixture read");
    serde_json::from_str(&raw).expect("fixture parse")
}

#[test]
fn classify_valid_fixtures() {
    let response = load_json("tests/fixtures/valid/response.json");
    let server_request = load_json("tests/fixtures/valid/server_request.json");
    let notification = load_json("tests/fixtures/valid/notification.json");

    assert_eq!(classify_message(&response), MsgKind::Response);
    assert_eq!(classify_message(&server_request), MsgKind::ServerRequest);
    assert_eq!(classify_message(&notification), MsgKind::Notification);
}

#[test]
fn classify_edge_fixture() {
    let unknown = load_json("tests/fixtures/edge/unknown.json");
    assert_eq!(classify_message(&unknown), MsgKind::Unknown);
}

#[test]
fn extract_ids_from_server_request_fixture() {
    let server_request = load_json("tests/fixtures/valid/server_request.json");
    let ids = extract_ids(&server_request);
    assert_eq!(ids.thread_id.as_deref(), Some("thr_1"));
    assert_eq!(ids.turn_id.as_deref(), Some("turn_1"));
    assert_eq!(ids.item_id.as_deref(), Some("item_1"));
}

#[test]
fn invalid_fixture_is_rejected_by_parser() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/invalid/not_json.txt");
    let raw = std::fs::read_to_string(path).expect("fixture read");
    assert!(serde_json::from_str::<serde_json::Value>(&raw).is_err());
}

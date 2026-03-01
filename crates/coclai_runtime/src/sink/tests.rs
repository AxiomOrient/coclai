use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use super::*;
use crate::events::{Direction, MsgKind};

fn temp_file_path() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("coclai_runtime_sink_{nanos}.jsonl"))
}

#[tokio::test(flavor = "current_thread")]
async fn jsonl_file_sink_writes_one_line_per_envelope() {
    let path = temp_file_path();
    let sink = JsonlFileSink::open(&path).await.expect("open sink");

    let envelope = Envelope {
        seq: 1,
        ts_millis: 0,
        direction: Direction::Inbound,
        kind: MsgKind::Notification,
        rpc_id: None,
        method: Some("turn/started".to_owned()),
        thread_id: Some("thr_1".to_owned()),
        turn_id: Some("turn_1".to_owned()),
        item_id: None,
        json: json!({"method":"turn/started","params":{"threadId":"thr_1","turnId":"turn_1"}}),
    };

    sink.on_envelope(&envelope).await.expect("write envelope");

    let contents = fs::read_to_string(&path).expect("read sink file");
    let line = contents.trim_end();
    assert!(!line.is_empty(), "sink line must not be empty");
    let parsed: Envelope = serde_json::from_str(line).expect("valid envelope json");
    assert_eq!(parsed.seq, 1);
    assert_eq!(parsed.method.as_deref(), Some("turn/started"));

    let _ = fs::remove_file(path);
}

use super::*;

#[tokio::test(flavor = "current_thread")]
async fn serialize_envelope_to_sse() {
    let envelope = Envelope {
        seq: 1,
        ts_millis: 0,
        direction: Direction::Inbound,
        kind: MsgKind::Response,
        rpc_id: Some(JsonRpcId::Number(777)),
        method: None,
        thread_id: Some(Arc::from("thr_1")),
        turn_id: Some(Arc::from("turn_1")),
        item_id: None,
        json: Arc::new(json!({"id":777,"result":{"ok":true}})),
    };

    let sse = serialize_sse_envelope(&envelope).expect("serialize");
    assert!(sse.starts_with("data: {"));
    assert!(sse.ends_with("\n\n"));
    let payload = sse
        .strip_prefix("data: ")
        .and_then(|line| line.strip_suffix("\n\n"))
        .expect("sse payload framing");
    assert!(
        !payload.contains("\"rpcId\""),
        "external SSE payload must not expose internal rpc id"
    );
    let json_payload: Value = serde_json::from_str(payload).expect("parse sse json payload");
    assert!(
        json_payload["json"].get("id").is_none(),
        "response json must not expose internal rpc id"
    );
}

#[test]
fn parse_turn_id_from_turn_result_supports_common_shapes() {
    assert_eq!(
        wire::parse_turn_id_from_turn_result(&json!({"turn":{"id":"turn_nested"}})),
        Some("turn_nested".to_owned())
    );
    assert_eq!(
        wire::parse_turn_id_from_turn_result(&json!({"turnId":"turn_field"})),
        Some("turn_field".to_owned())
    );
    assert_eq!(
        wire::parse_turn_id_from_turn_result(&json!({"id":"turn_top"})),
        None
    );
    assert_eq!(
        wire::parse_turn_id_from_turn_result(&json!("turn_raw")),
        None
    );
}

#[test]
fn extract_thread_id_from_server_request_params_supports_common_shapes() {
    assert_eq!(
        wire::extract_thread_id_from_server_request_params(&json!({"threadId":"thr_direct"})),
        Some("thr_direct".to_owned())
    );
    assert_eq!(
        wire::extract_thread_id_from_server_request_params(&json!({"thread":{"id":"thr_nested"}})),
        Some("thr_nested".to_owned())
    );
    assert_eq!(
        wire::extract_thread_id_from_server_request_params(
            &json!({"params":{"threadId":"thr_in_params"}})
        ),
        Some("thr_in_params".to_owned())
    );
    assert_eq!(
        wire::extract_thread_id_from_server_request_params(
            &json!({"params":{"thread":{"id":"thr_nested_params"}}})
        ),
        Some("thr_nested_params".to_owned())
    );
}

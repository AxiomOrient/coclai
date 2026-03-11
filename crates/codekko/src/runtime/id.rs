use serde_json::Value;

/// Canonical non-empty ID parsing.
/// Rejects empty values and strings with leading/trailing whitespace.
pub(crate) fn parse_canonical_id(value: &Value) -> Option<&str> {
    let raw = value.as_str()?;
    if raw.is_empty() || raw != raw.trim() {
        return None;
    }
    Some(raw)
}

/// Parse `threadId` from a known RPC result shape.
/// Accepts `{thread:{id}}` or `{threadId}`.
pub(crate) fn parse_result_thread_id(value: &Value) -> Option<&str> {
    parse_result_id(value, "/thread/id", "threadId")
}

/// Parse `turnId` from a known RPC result shape.
/// Accepts `{turn:{id}}` or `{turnId}`.
pub(crate) fn parse_result_turn_id(value: &Value) -> Option<&str> {
    parse_result_id(value, "/turn/id", "turnId")
}

/// Extract `threadId` from event/server-request style payload roots.
/// Accepts `threadId` or nested `thread.id`, with optional `params` nesting.
pub(crate) fn extract_thread_id(root: &Value) -> Option<&str> {
    extract_id_field(root, "threadId", "thread")
}

/// Extract `turnId` from event/server-request style payload roots.
/// Accepts `turnId` or nested `turn.id`, with optional `params` nesting.
pub(crate) fn extract_turn_id(root: &Value) -> Option<&str> {
    extract_id_field(root, "turnId", "turn")
}

/// Extract `itemId` from event/server-request style payload roots.
/// Accepts `itemId` or nested `item.id`, with optional `params` nesting.
pub(crate) fn extract_item_id(root: &Value) -> Option<&str> {
    extract_id_field(root, "itemId", "item")
}

fn parse_result_id<'a>(
    value: &'a Value,
    nested_pointer: &str,
    camel_id_field: &str,
) -> Option<&'a str> {
    value
        .pointer(nested_pointer)
        .and_then(parse_canonical_id)
        .or_else(|| value.get(camel_id_field).and_then(parse_canonical_id))
}

fn extract_id_field<'a>(
    root: &'a Value,
    camel_id_field: &str,
    nested_object_field: &str,
) -> Option<&'a str> {
    root.get(camel_id_field)
        .and_then(parse_canonical_id)
        .or_else(|| {
            root.get(nested_object_field)
                .and_then(|value| value.get("id"))
                .and_then(parse_canonical_id)
        })
        .or_else(|| {
            root.get("params")
                .and_then(|value| value.get(camel_id_field))
                .and_then(parse_canonical_id)
        })
        .or_else(|| {
            root.get("params")
                .and_then(|value| value.get(nested_object_field))
                .and_then(|value| value.get("id"))
                .and_then(parse_canonical_id)
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parse_result_ids_accept_nested_and_camel_shapes() {
        assert_eq!(
            parse_result_thread_id(&json!({"thread":{"id":"thr_nested"}})),
            Some("thr_nested")
        );
        assert_eq!(
            parse_result_turn_id(&json!({"turnId":"turn_field"})),
            Some("turn_field")
        );
    }

    #[test]
    fn parse_result_ids_reject_loose_and_invalid_shapes() {
        assert_eq!(parse_result_thread_id(&json!({"id":"thr_top"})), None);
        assert_eq!(parse_result_turn_id(&json!("turn_raw")), None);
        assert_eq!(parse_result_thread_id(&json!({"threadId":""})), None);
        assert_eq!(parse_result_turn_id(&json!({"turn":{"id":"  "}})), None);
    }

    #[test]
    fn extract_ids_supports_direct_nested_and_params_shapes() {
        assert_eq!(
            extract_thread_id(&json!({"threadId":"thr_direct"})),
            Some("thr_direct")
        );
        assert_eq!(
            extract_thread_id(&json!({"thread":{"id":"thr_nested"}})),
            Some("thr_nested")
        );
        assert_eq!(
            extract_turn_id(&json!({"params":{"turnId":"turn_params"}})),
            Some("turn_params")
        );
        assert_eq!(
            extract_item_id(&json!({"params":{"item":{"id":"item_nested"}}})),
            Some("item_nested")
        );
    }
}

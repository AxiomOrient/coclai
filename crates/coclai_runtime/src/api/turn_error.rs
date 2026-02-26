use serde_json::Value;

use crate::events::Envelope;

use super::{PromptTurnFailure, PromptTurnTerminalState};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PromptTurnErrorSignal {
    source_method: String,
    code: Option<i64>,
    message: String,
}

impl PromptTurnErrorSignal {
    pub(super) fn into_failure(self, terminal_state: PromptTurnTerminalState) -> PromptTurnFailure {
        PromptTurnFailure {
            terminal_state,
            source_method: self.source_method,
            code: self.code,
            message: self.message,
        }
    }
}

/// Extract turn-scoped error signal from one envelope.
/// Allocation: one signal struct only when error exists. Complexity: O(1).
pub(super) fn extract_turn_error_signal(envelope: &Envelope) -> Option<PromptTurnErrorSignal> {
    let method = envelope.method.as_deref()?;
    if method != "error" && method != "turn/failed" {
        return None;
    }

    let params = envelope.json.get("params");
    let roots = [
        params.and_then(|v| v.get("error")),
        envelope.json.get("error"),
        params,
        Some(&envelope.json),
    ];

    for root in roots.into_iter().flatten() {
        if let Some((code, message)) = extract_error_message(root) {
            return Some(PromptTurnErrorSignal {
                source_method: method.to_owned(),
                code,
                message,
            });
        }
    }

    Some(PromptTurnErrorSignal {
        source_method: method.to_owned(),
        code: None,
        message: format!("{method} event"),
    })
}

/// Extract one human-readable error message from a generic JSON payload.
/// Allocation: one String only on match. Complexity: O(1).
fn extract_error_message(root: &Value) -> Option<(Option<i64>, String)> {
    let message = root
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| root.get("detail").and_then(Value::as_str))
        .or_else(|| root.get("reason").and_then(Value::as_str))
        .or_else(|| root.get("text").and_then(Value::as_str))
        .or_else(|| {
            root.get("error")
                .and_then(|v| v.get("message"))
                .and_then(Value::as_str)
        })?;

    let code = root.get("code").and_then(Value::as_i64);
    Some((code, message.to_owned()))
}

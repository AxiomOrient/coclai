use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

use crate::errors::RpcError;
use crate::turn_output::parse_turn_id;

use super::wire::{input_item_to_wire, turn_start_params_to_wire, validate_turn_start_security};
use super::{InputItem, ThreadHandle, TurnHandle, TurnStartParams};

pub(super) fn serialize_params<T: Serialize>(method: &str, params: &T) -> Result<Value, RpcError> {
    serde_json::to_value(params)
        .map_err(|error| RpcError::InvalidRequest(format!("{method} invalid params: {error}")))
}

pub(super) fn deserialize_result<T: DeserializeOwned>(
    method: &str,
    response: Value,
) -> Result<T, RpcError> {
    serde_json::from_value(response.clone()).map_err(|error| {
        RpcError::InvalidRequest(format!(
            "{method} invalid result: {error}; response: {response}"
        ))
    })
}

impl ThreadHandle {
    pub fn runtime(&self) -> &crate::runtime::Runtime {
        &self.runtime
    }

    pub async fn turn_start(&self, p: TurnStartParams) -> Result<TurnHandle, RpcError> {
        if p.input.is_empty() {
            return Err(RpcError::InvalidRequest(
                "turn input must not be empty".to_owned(),
            ));
        }
        validate_turn_start_security(&p)?;

        let response = self
            .runtime
            .call_raw("turn/start", turn_start_params_to_wire(&self.thread_id, &p))
            .await?;

        let turn_id = parse_turn_id(&response).ok_or_else(|| {
            RpcError::InvalidRequest(format!("turn/start missing turn id in result: {response}"))
        })?;

        Ok(TurnHandle {
            turn_id,
            thread_id: self.thread_id.clone(),
        })
    }

    /// Start a follow-up turn anchored to an expected previous turn id.
    /// Allocation: JSON params + input item wire objects.
    /// Complexity: O(n), n = input item count.
    pub async fn turn_steer(
        &self,
        expected_turn_id: &str,
        input: Vec<InputItem>,
    ) -> Result<super::TurnId, RpcError> {
        if input.is_empty() {
            return Err(RpcError::InvalidRequest(
                "turn input must not be empty".to_owned(),
            ));
        }

        let mut params = Map::<String, Value>::new();
        params.insert("threadId".to_owned(), Value::String(self.thread_id.clone()));
        params.insert(
            "expectedTurnId".to_owned(),
            Value::String(expected_turn_id.to_owned()),
        );
        params.insert(
            "input".to_owned(),
            Value::Array(input.iter().map(input_item_to_wire).collect()),
        );
        let response = self
            .runtime
            .call_raw("turn/start", Value::Object(params))
            .await?;
        parse_turn_id(&response).ok_or_else(|| {
            RpcError::InvalidRequest(format!(
                "turn/start(steer) missing turn id in result: {response}"
            ))
        })
    }

    pub async fn turn_interrupt(&self, turn_id: &str) -> Result<(), RpcError> {
        self.runtime.turn_interrupt(&self.thread_id, turn_id).await
    }
}

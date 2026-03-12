use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

use crate::runtime::{
    Client, ClientConfig, ClientError, RpcError, RpcErrorObject, RpcValidationMode, RuntimeError,
    ServerRequestRx,
};

pub(super) async fn connect(config: ClientConfig) -> Result<Client, ClientError> {
    Client::connect(config).await
}

pub(super) async fn connect_default() -> Result<Client, ClientError> {
    Client::connect_default().await
}

pub(super) async fn request_json(
    client: &Client,
    method: &str,
    params: Value,
    mode: RpcValidationMode,
) -> Result<Value, RpcError> {
    client
        .runtime()
        .call_validated_with_mode(method, params, mode)
        .await
}

pub(super) async fn request_typed<P, R>(
    client: &Client,
    method: &str,
    params: P,
    mode: RpcValidationMode,
) -> Result<R, RpcError>
where
    P: Serialize,
    R: DeserializeOwned,
{
    client
        .runtime()
        .call_typed_validated_with_mode(method, params, mode)
        .await
}

pub(super) async fn request_json_unchecked(
    client: &Client,
    method: &str,
    params: Value,
) -> Result<Value, RpcError> {
    client.runtime().call_raw(method, params).await
}

pub(super) async fn notify_json(
    client: &Client,
    method: &str,
    params: Value,
    mode: RpcValidationMode,
) -> Result<(), RuntimeError> {
    client
        .runtime()
        .notify_validated_with_mode(method, params, mode)
        .await
}

pub(super) async fn notify_typed<P>(
    client: &Client,
    method: &str,
    params: P,
    mode: RpcValidationMode,
) -> Result<(), RuntimeError>
where
    P: Serialize,
{
    client
        .runtime()
        .notify_typed_validated_with_mode(method, params, mode)
        .await
}

pub(super) async fn notify_json_unchecked(
    client: &Client,
    method: &str,
    params: Value,
) -> Result<(), RuntimeError> {
    client.runtime().notify_raw(method, params).await
}

pub(super) async fn take_server_requests(client: &Client) -> Result<ServerRequestRx, RuntimeError> {
    client.runtime().take_server_request_rx().await
}

pub(super) async fn respond_server_request_ok(
    client: &Client,
    approval_id: &str,
    result: Value,
) -> Result<(), RuntimeError> {
    client
        .runtime()
        .respond_approval_ok(approval_id, result)
        .await
}

pub(super) async fn respond_server_request_err(
    client: &Client,
    approval_id: &str,
    err: RpcErrorObject,
) -> Result<(), RuntimeError> {
    client
        .runtime()
        .respond_approval_err(approval_id, err)
        .await
}

pub(super) async fn shutdown(client: &Client) -> Result<(), RuntimeError> {
    client.shutdown().await
}

use serde::Serialize;

use crate::bindings::tauri::{invoke, invoke_no_args};
use crate::models::{AppError, ConnectionInput, SshConnection};

#[derive(Serialize)]
struct IdArgs<'a> {
    id: &'a str,
}

#[derive(Serialize)]
struct InputArgs<'a> {
    input: &'a ConnectionInput,
}

#[derive(Serialize)]
struct IdInputArgs<'a> {
    id: &'a str,
    input: &'a ConnectionInput,
}

pub async fn list_connections() -> Result<Vec<SshConnection>, AppError> {
    invoke_no_args("list_connections").await
}

pub async fn create_connection(input: &ConnectionInput) -> Result<SshConnection, AppError> {
    invoke("create_connection", &InputArgs { input }).await
}

pub async fn update_connection(
    id: &str,
    input: &ConnectionInput,
) -> Result<SshConnection, AppError> {
    invoke("update_connection", &IdInputArgs { id, input }).await
}

pub async fn delete_connection(id: &str) -> Result<(), AppError> {
    invoke("delete_connection", &IdArgs { id }).await
}

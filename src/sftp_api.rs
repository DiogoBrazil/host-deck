//! Thin wrappers over the SFTP Tauri commands (mirrors `api.rs`).
//!
//! Session lifecycle (connect, host-key prompt, disconnect) and native file
//! dialogs live in the JS bridge (`public/js/sftp.js`); these are the typed,
//! request/response operations invoked directly from WASM.

use serde::Serialize;

use crate::bindings::tauri::invoke;
use crate::models::{AppError, RemoteEntry};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionPath<'a> {
    session_id: &'a str,
    path: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RenameArgs<'a> {
    session_id: &'a str,
    from: &'a str,
    to: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadArgs<'a> {
    session_id: &'a str,
    transfer_id: &'a str,
    remote_path: &'a str,
    local_path: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadArgs<'a> {
    session_id: &'a str,
    transfer_id: &'a str,
    local_path: &'a str,
    remote_path: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelArgs<'a> {
    session_id: &'a str,
    transfer_id: &'a str,
}

pub async fn realpath(session_id: &str, path: &str) -> Result<String, AppError> {
    invoke("sftp_realpath", &SessionPath { session_id, path }).await
}

pub async fn list_dir(session_id: &str, path: &str) -> Result<Vec<RemoteEntry>, AppError> {
    invoke("sftp_list_dir", &SessionPath { session_id, path }).await
}

pub async fn mkdir(session_id: &str, path: &str) -> Result<(), AppError> {
    invoke("sftp_mkdir", &SessionPath { session_id, path }).await
}

pub async fn rename(session_id: &str, from: &str, to: &str) -> Result<(), AppError> {
    invoke("sftp_rename", &RenameArgs { session_id, from, to }).await
}

pub async fn remove_file(session_id: &str, path: &str) -> Result<(), AppError> {
    invoke("sftp_remove_file", &SessionPath { session_id, path }).await
}

pub async fn remove_dir(session_id: &str, path: &str) -> Result<(), AppError> {
    invoke("sftp_remove_dir", &SessionPath { session_id, path }).await
}

pub async fn download(
    session_id: &str,
    transfer_id: &str,
    remote_path: &str,
    local_path: &str,
) -> Result<(), AppError> {
    invoke(
        "sftp_download",
        &DownloadArgs {
            session_id,
            transfer_id,
            remote_path,
            local_path,
        },
    )
    .await
}

pub async fn upload(
    session_id: &str,
    transfer_id: &str,
    local_path: &str,
    remote_path: &str,
) -> Result<(), AppError> {
    invoke(
        "sftp_upload",
        &UploadArgs {
            session_id,
            transfer_id,
            local_path,
            remote_path,
        },
    )
    .await
}

pub async fn cancel_transfer(session_id: &str, transfer_id: &str) -> Result<(), AppError> {
    invoke(
        "sftp_cancel_transfer",
        &CancelArgs {
            session_id,
            transfer_id,
        },
    )
    .await
}

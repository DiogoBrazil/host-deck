mod agent;
mod commands;
mod domain;
mod error;
mod infra;
mod sftp;
mod ssh;
mod state;

use std::sync::Arc;

use tauri::Manager;

use agent::registry::AgentRegistry;
use commands::{agent_commands, connection_commands, sftp_commands, terminal_commands};
use infra::credential_store::SystemKeyring;
use infra::db::Db;
use sftp::registry::SftpRegistry;
use ssh::registry::SessionRegistry;
use state::CredStore;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let db_path = app.path().app_data_dir()?.join("hostdeck.db");
            let db = Db::open(&db_path)?;
            app.manage(db);
            app.manage(CredStore(Arc::new(SystemKeyring::new())));
            app.manage(SessionRegistry::default());
            app.manage(SftpRegistry::default());
            app.manage(AgentRegistry::default());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            connection_commands::list_connections,
            connection_commands::get_connection,
            connection_commands::create_connection,
            connection_commands::update_connection,
            connection_commands::delete_connection,
            terminal_commands::ssh_connect,
            terminal_commands::ssh_connect_with_password,
            terminal_commands::ssh_send_data,
            terminal_commands::ssh_resize,
            terminal_commands::ssh_disconnect,
            terminal_commands::confirm_host_key,
            sftp_commands::sftp_connect,
            sftp_commands::sftp_connect_with_password,
            sftp_commands::sftp_realpath,
            sftp_commands::sftp_list_dir,
            sftp_commands::sftp_download,
            sftp_commands::sftp_upload,
            sftp_commands::sftp_mkdir,
            sftp_commands::sftp_rename,
            sftp_commands::sftp_remove_file,
            sftp_commands::sftp_remove_dir,
            sftp_commands::sftp_cancel_transfer,
            sftp_commands::sftp_disconnect,
            agent_commands::agent_send,
            agent_commands::agent_cancel,
            agent_commands::confirm_agent_command,
            agent_commands::agent_refresh_models,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

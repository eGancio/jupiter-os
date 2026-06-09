// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

mod autostart;
mod chat_commands;
mod chat_state;
mod claude;
mod commands;
mod config;
mod credentials;
mod emails;
mod europa_credentials;
mod metis;
mod services;
mod state;

use serde::Serialize;
use chat_state::ChatState;
use state::AppState;
use tauri::{Emitter, Manager};

#[derive(Clone, Serialize)]
struct StatusChanged {
    name: String,
    running: bool,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .manage(ChatState::new())
        .manage(europa_credentials::TelegramAuth::default())
        .manage(europa_credentials::TeamsAuth::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_services,
            commands::get_virtual_moons,
            commands::start_service,
            commands::stop_service,
            commands::restart_service,
            commands::start_all,
            commands::stop_all,
            commands::get_logs,
            commands::clear_logs,
            commands::is_autostart_enabled,
            commands::set_autostart,
            commands::add_local_moon,
            commands::add_remote_moon,
            commands::remove_moon,
            commands::remove_local_moon,
            chat_commands::create_chat_session,
            chat_commands::send_chat_message,
            chat_commands::stop_chat,
            chat_commands::get_chat_messages,
            chat_commands::list_chat_sessions,
            chat_commands::set_chat_model,
            chat_commands::set_chat_permission_mode,
            chat_commands::delete_chat_session,
            chat_commands::compact_chat_session,
            chat_commands::rename_chat_session,
            chat_commands::get_chat_model,
            chat_commands::get_chat_engine,
            chat_commands::set_chat_engine,
            emails::list_emails_io,
            metis::metis_ingest_paths,
            metis::metis_list_documents,
            metis::metis_delete_document,
            metis::metis_open_file,
            metis::metis_reveal_file,
            chat_commands::get_claude_md_status,
            chat_commands::read_chart_file,
            chat_commands::flush_session_messages,
            credentials::credentials_list,
            credentials::credentials_set,
            credentials::credentials_delete,
            credentials::credentials_add_account,
            credentials::oauth_status,
            credentials::oauth_disconnect,
            credentials::oauth_connect_gmail,
            credentials::account_remove,
            europa_credentials::europa_channels_list,
            europa_credentials::telegram_save_api,
            europa_credentials::telegram_request_code,
            europa_credentials::telegram_submit_code,
            europa_credentials::telegram_submit_password,
            europa_credentials::telegram_save_bot,
            europa_credentials::slack_save,
            europa_credentials::teams_start_device_code,
            europa_credentials::teams_poll_device_code,
            europa_credentials::europa_channel_remove,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // Auto-start daemons
            let state = app.state::<AppState>();
            state.autostart_daemons(handle.clone());

            // Initialize Agent SDK sidecar
            let chat_state = app.state::<ChatState>();
            chat_state.init_sidecar(handle.clone());

            // Background health-check thread (every 500ms)
            let services = state.services.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let mut changed = Vec::new();
                if let Ok(ref mut svcs) = services.lock() {
                    for (_name, svc) in svcs.iter_mut() {
                        if svc.check_alive() {
                            changed.push(StatusChanged {
                                name: svc.name.clone(),
                                running: svc.running,
                            });
                        }
                    }
                }
                if !changed.is_empty() {
                    let _ = handle.emit("service-status-changed", &changed);
                }
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                let state = app_handle.state::<AppState>();
                state.stop_all();

                // Save sessions and stop the sidecar on exit
                let chat_state = app_handle.state::<ChatState>();
                chat_state.save_sessions();
                chat_state.stop_sidecar();
            }
        });
}

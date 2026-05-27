mod autostart;
mod chat_commands;
mod chat_state;
mod claude;
mod commands;
mod config;
mod credentials;
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
        .manage(AppState::new())
        .manage(ChatState::new())
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
            chat_commands::create_chat_session,
            chat_commands::send_chat_message,
            chat_commands::stop_chat,
            chat_commands::get_chat_messages,
            chat_commands::list_chat_sessions,
            chat_commands::set_chat_model,
            chat_commands::delete_chat_session,
            chat_commands::compact_chat_session,
            chat_commands::rename_chat_session,
            chat_commands::get_chat_model,
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

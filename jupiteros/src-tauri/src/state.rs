use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tauri::AppHandle;

use crate::config::{load_credentials_env, mcp_json_path, McpConfig, ProcessKind};
use crate::services::ServiceState;

pub struct AppState {
    pub services: Arc<Mutex<BTreeMap<String, ServiceState>>>,
}

impl AppState {
    pub fn new() -> Self {
        let json_path = mcp_json_path();
        let config: McpConfig = std::fs::read_to_string(&json_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| McpConfig {
                _mcp_servers: BTreeMap::new(),
                servers: BTreeMap::new(),
                daemons: BTreeMap::new(),
            });

        let creds = load_credentials_env();
        let mut services = BTreeMap::new();

        for (name, mut def) in config.servers {
            // Merge credentials as defaults (mcp.json values win)
            for (k, v) in &creds {
                def.env.entry(k.clone()).or_insert_with(|| v.clone());
            }
            services.insert(name.clone(), ServiceState::new(name, def, ProcessKind::McpServer));
        }

        for (name, mut def) in config.daemons {
            for (k, v) in &creds {
                def.env.entry(k.clone()).or_insert_with(|| v.clone());
            }
            services.insert(name.clone(), ServiceState::new(name, def, ProcessKind::Daemon));
        }

        Self {
            services: Arc::new(Mutex::new(services)),
        }
    }

    pub fn autostart_daemons(&self, app_handle: AppHandle) {
        if let Ok(mut services) = self.services.lock() {
            for state in services.values_mut() {
                if state.kind == ProcessKind::Daemon && !state.running {
                    state.start(Some(app_handle.clone()));
                }
            }
        }
    }

    pub fn stop_all(&self) {
        if let Ok(mut services) = self.services.lock() {
            for state in services.values_mut() {
                if state.running {
                    state.stop();
                }
            }
        }
    }
}

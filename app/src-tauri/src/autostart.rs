// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::path::PathBuf;

fn startup_shortcut_path() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_default();
    PathBuf::from(appdata)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join("JupiterOS.lnk")
}

pub fn is_autostart_enabled() -> bool {
    startup_shortcut_path().exists()
}

pub fn set_autostart(enabled: bool) {
    let shortcut = startup_shortcut_path();
    if enabled {
        if let Ok(exe) = std::env::current_exe() {
            let target = exe.to_string_lossy().replace('/', "\\");
            let working_dir = exe
                .parent()
                .map(|p| p.to_string_lossy().replace('/', "\\"))
                .unwrap_or_default();
            let shortcut_path = shortcut.to_string_lossy().replace('/', "\\");

            let ps_script = format!(
                "$ws = New-Object -ComObject WScript.Shell; \
                 $sc = $ws.CreateShortcut('{}'); \
                 $sc.TargetPath = '{}'; \
                 $sc.WorkingDirectory = '{}'; \
                 $sc.Save()",
                shortcut_path, target, working_dir
            );

            let mut cmd = std::process::Command::new("powershell");
            cmd.args(["-NoProfile", "-Command", &ps_script]);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            }
            let _ = cmd.output();
        }
    } else if shortcut.exists() {
        let _ = std::fs::remove_file(&shortcut);
    }
}

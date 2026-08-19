//! The command surface exposed to the UI. Everything here is a thin wrapper:
//! the logic lives in `hardware`, `gpu` and `config` so it stays testable
//! outside a running window.

use std::path::PathBuf;

use tauri::{AppHandle, Manager, State};

use crate::config;
use crate::models::{HardwareSnapshot, SharingConfig};
use crate::AgentState;

#[tauri::command]
pub fn get_hardware_info(state: State<'_, AgentState>) -> HardwareSnapshot {
    state.hardware.clone()
}

#[tauri::command]
pub fn load_sharing_config(
    app: AppHandle,
    state: State<'_, AgentState>,
) -> Result<SharingConfig, String> {
    let dir = config_dir(&app)?;

    config::load_for_hardware(&dir, &state.hardware).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_sharing_config(
    app: AppHandle,
    state: State<'_, AgentState>,
    config: SharingConfig,
) -> Result<SharingConfig, String> {
    let dir = config_dir(&app)?;
    // The webview is treated as untrusted input: caps are re-clamped here even
    // though the UI already limits its own inputs.
    let clamped = config.clamped_to(&state.hardware);

    config::save(&dir, &clamped).map_err(|error| error.to_string())?;

    Ok(clamped)
}

#[tauri::command]
pub fn get_config_file_path(app: AppHandle) -> Result<String, String> {
    Ok(config::config_file_path(&config_dir(&app)?)
        .to_string_lossy()
        .into_owned())
}

fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|error| format!("could not resolve the app config directory: {error}"))
}

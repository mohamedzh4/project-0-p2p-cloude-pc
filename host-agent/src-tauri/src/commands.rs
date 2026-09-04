//! The command surface exposed to the UI. Everything here is a thin wrapper:
//! the logic lives in `hardware`, `gpu`, `config` and `matchmaker` so it stays
//! testable outside a running window.

use std::path::PathBuf;

use tauri::{AppHandle, Manager, State};

use crate::agent_status::AgentStatus;
use crate::config;
use crate::identity::{self, HostRegistration};
use crate::matchmaker::{self, UreqTransport};
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
    crate::tray::refresh(&app);

    Ok(clamped)
}

#[tauri::command]
pub fn get_config_file_path(app: AppHandle) -> Result<String, String> {
    Ok(config::config_file_path(&config_dir(&app)?)
        .to_string_lossy()
        .into_owned())
}

#[tauri::command]
pub fn load_host_registration(app: AppHandle) -> Result<Option<HostRegistration>, String> {
    identity::load(&config_dir(&app)?).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_agent_status(
    app: AppHandle,
    state: State<'_, AgentState>,
) -> Result<AgentStatus, String> {
    let dir = config_dir(&app)?;
    let config =
        config::load_for_hardware(&dir, &state.hardware).map_err(|error| error.to_string())?;
    Ok(AgentStatus::from_flags(
        config.sharing_enabled,
        state.occupancy.is_in_session(),
    ))
}

/// FR11: write `sharingEnabled` immediately so the heartbeat loop sees pause
/// on the next tick. Does not take slider drafts from the webview.
#[tauri::command]
pub fn set_sharing_enabled(
    app: AppHandle,
    state: State<'_, AgentState>,
    enabled: bool,
) -> Result<SharingConfig, String> {
    let dir = config_dir(&app)?;
    let stored = config::set_sharing_enabled(&dir, &state.hardware, enabled)
        .map_err(|error| error.to_string())?;
    crate::tray::refresh(&app);
    Ok(stored)
}

/// Reads the on-disk config (not the webview payload), clamps again, then
/// POST/PUT `/v1/hosts`. Fails if Matchmaker URL or bootstrap token is missing.
#[tauri::command]
pub async fn register_declared_caps(
    app: AppHandle,
    state: State<'_, AgentState>,
) -> Result<HostRegistration, String> {
    let dir = config_dir(&app)?;
    let hardware = state.hardware.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let registration = matchmaker::register_from_env(&dir, &hardware, &UreqTransport)
            .map_err(|error| error.to_string())?;
        if let Err(error) = matchmaker::heartbeat_from_env(&dir, &UreqTransport) {
            eprintln!("host-agent heartbeat after register: {error}");
        }
        Ok(registration)
    })
    .await
    .map_err(|error| format!("could not run host registration: {error}"))?
}

fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|error| format!("could not resolve the app config directory: {error}"))
}

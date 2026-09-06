//! The command surface exposed to the UI. Everything here is a thin wrapper:
//! the logic lives in `hardware`, `gpu`, `config` and `matchmaker` so it stays
//! testable outside a running window.

use std::path::PathBuf;

use tauri::{AppHandle, Manager, State};

use crate::agent_status::AgentStatus;
use crate::config;
use crate::eligibility::{self, EligibilityReport, EligibilityRequest};
use crate::gpu_bind::{self, GpuBindStatus, PciSysfs, SysfsBindOps};
use crate::host_capabilities::HostCapabilities;
use crate::hypervisor::HypervisorStatus;
use crate::identity::{self, HostRegistration};
use crate::image::GuestImageStatus;
use crate::launch::{self, LaunchConfig};
use crate::matchmaker::{self, UreqTransport};
use crate::mesh::{self, ProcessJoinRunner};
use crate::models::{HardwareSnapshot, SharingConfig};
use crate::provision::{HostOs, LabTier};
use crate::AgentState;

#[tauri::command]
pub fn get_hardware_info(state: State<'_, AgentState>) -> HardwareSnapshot {
    state.hardware.clone()
}

#[tauri::command]
pub fn get_host_capabilities(state: State<'_, AgentState>) -> HostCapabilities {
    state.capabilities.clone()
}

#[tauri::command]
pub fn get_hypervisor_status(state: State<'_, AgentState>) -> HypervisorStatus {
    state.hypervisor.clone()
}

#[tauri::command]
pub fn get_guest_image_status(state: State<'_, AgentState>) -> GuestImageStatus {
    state.guest_image.clone()
}

/// Map the webview's sliders (re-clamped) to a launch document. Never boots.
#[tauri::command]
pub fn build_launch_config(
    state: State<'_, AgentState>,
    config: SharingConfig,
    lab_tier: String,
) -> Result<LaunchConfig, String> {
    let lab_tier = LabTier::parse(&lab_tier).map_err(|error| error.to_string())?;
    let cache = crate::image::resolve_image_cache_dir(|key| std::env::var(key).ok());
    let spare_disk_bytes = crate::disk_space::available_bytes(&cache);
    launch::build(launch::LaunchRequest {
        os: HostOs::current(),
        lab_tier,
        hardware: &state.hardware,
        sharing: &config,
        image_cache_dir: Some(cache.as_path()),
        guest_image: Some(&state.guest_image),
        capabilities: Some(&state.capabilities),
        spare_disk_bytes,
    })
    .map_err(|error| error.to_string())
}

/// Combined Standard/Pro offer from the probe, sliders, spare disk, and VFIO
/// plan. A blocked tier is not offered. Never boots.
#[tauri::command]
pub fn evaluate_hosting_offers(
    state: State<'_, AgentState>,
    config: SharingConfig,
) -> EligibilityReport {
    let cache = crate::image::resolve_image_cache_dir(|key| std::env::var(key).ok());
    let spare_disk_bytes = crate::disk_space::available_bytes(&cache);
    let gpu_bind = gpu_bind::report(
        gpu_bind::BindRequest {
            os: HostOs::current(),
            capabilities: &state.capabilities,
            share_gpu: config.share_gpu,
            shared_gpu_id: config.shared_gpu_id.as_deref(),
            sysfs: &PciSysfs::live(),
            apply: false,
            hyperv: None,
            hyperv_vm_name: None,
        },
        &SysfsBindOps,
    );
    eligibility::evaluate(EligibilityRequest {
        capabilities: &state.capabilities,
        hardware: &state.hardware,
        sharing: &config,
        spare_disk_bytes,
        gpu_bind: Some(&gpu_bind),
    })
}

/// Plan Pro GPU assignment from the probe + sliders. Never binds live sysfs,
/// never spawns PowerShell, and never invokes a hypervisor. Windows apply
/// (DDA/GPU-P) is leftover test/injectable path, not this command.
#[tauri::command]
pub fn plan_gpu_bind(state: State<'_, AgentState>, config: SharingConfig) -> GpuBindStatus {
    gpu_bind::report(
        gpu_bind::BindRequest {
            os: HostOs::current(),
            capabilities: &state.capabilities,
            share_gpu: config.share_gpu,
            shared_gpu_id: config.shared_gpu_id.as_deref(),
            sysfs: &PciSysfs::live(),
            apply: false,
            hyperv: None,
            hyperv_vm_name: None,
        },
        &SysfsBindOps,
    )
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
pub async fn get_agent_status(
    app: AppHandle,
    state: State<'_, AgentState>,
) -> Result<AgentStatus, String> {
    let dir = config_dir(&app)?;
    let hardware = state.hardware.clone();
    let in_session = state.occupancy.is_in_session();

    tauri::async_runtime::spawn_blocking(move || {
        let config =
            config::load_for_hardware(&dir, &hardware).map_err(|error| error.to_string())?;
        let credits = matchmaker::fetch_credits_accrued_from_env(&dir, &UreqTransport).ok();
        Ok(AgentStatus::from_flags(
            config.sharing_enabled,
            in_session,
            credits,
        ))
    })
    .await
    .map_err(|error| format!("could not load agent status: {error}"))?
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
        if mesh::should_join(|key| std::env::var(key).ok()) {
            mesh::join_from_env(
                &dir,
                |key| std::env::var(key).ok(),
                &UreqTransport,
                &ProcessJoinRunner,
            )
            .map_err(|error| error.to_string())?;
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

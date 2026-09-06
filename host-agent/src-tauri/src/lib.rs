//! Detect the machine, persist local sharing caps, register those caps with
//! the Matchmaker from Rust, ping while sharing is on, and optionally join
//! self-hosted Headscale from Rust. Provisioning is a stub HTTP port that
//! calls `plan_provision`. Tray status and immediate pause live in Rust; the
//! webview does not call the Matchmaker.

mod agent_status;
mod commands;
mod config;
mod disk_space;
mod eligibility;
mod gpu;
mod gpu_bind;
mod hardware;
mod host_capabilities;
mod hypervisor;
mod identity;
mod image;
mod launch;
mod matchmaker;
mod mesh;
mod models;
mod provision;
mod provision_http;
mod storage;
mod tray;

#[cfg(test)]
mod test_support;

pub use eligibility::{EligibilityReport, MIN_DISK_BYTES};
pub use gpu_bind::GpuBindStatus;
pub use host_capabilities::{evaluate as evaluate_host_capabilities, probe_live, HostCapabilities};
pub use hypervisor::HypervisorStatus;
pub use image::{
    verify_fetched_bytes, verify_image, GuestImageStatus, ImageError, ImageManifest, ImageTrust,
};
pub use launch::{LaunchConfig, LaunchError};
pub use models::HardwareSnapshot;
/// Phase 6: Matchmaker calls the HTTP port; that port verifies the signed
/// image then calls [`plan_provision`]. Selection takes plain types (no Tauri)
/// so the listener can reuse it.
pub use provision::{
    plan as plan_provision, HostOs, LabTier as ProvisionLabTier, ProvisionError, ProvisionPlan,
    ProvisionRequest,
};

use std::sync::Arc;

use tauri::Manager;

use crate::agent_status::Occupancy;
use crate::matchmaker::UreqTransport;

/// Long-lived state shared by every command.
pub struct AgentState {
    /// Probed once at startup: the machine's hardware does not change while the
    /// agent runs, and re-probing per request would waste the resources the
    /// agent exists to lend out.
    pub hardware: HardwareSnapshot,
    /// Hosting eligibility from OS + KVM/WHPX/IOMMU/GPU. Never a backend picker.
    pub capabilities: HostCapabilities,
    /// Locate/fetch result for the OS hypervisor binary. Never invoked.
    pub hypervisor: HypervisorStatus,
    /// Locate/fetch result for the signed guest image. Never booted.
    pub guest_image: GuestImageStatus,
    /// Set when stub provision succeeds. Pause does not clear this; phase 12
    /// teardown will.
    pub occupancy: Arc<Occupancy>,
}

/// Reads this machine's hardware without opening a window, for diagnostics and
/// for the headless service this agent grows into.
pub fn detect_hardware() -> HardwareSnapshot {
    hardware::detect()
}

fn load_local_env() {
    // `tauri dev` usually starts in host-agent/; `cargo run` in src-tauri/.
    // Neither file is required — registration still fails fast if the vars
    // are missing after this attempt.
    let _ = dotenvy::from_filename(".env");
    let _ = dotenvy::from_filename("../.env");
}

fn spawn_heartbeat_loop(app: &tauri::App) {
    let handle = app.handle().clone();
    std::thread::Builder::new()
        .name("host-agent-heartbeat".into())
        .spawn(move || loop {
            if let Err(error) = tick_heartbeat(&handle) {
                eprintln!("host-agent heartbeat: {error}");
            }
            tray::refresh(&handle);
            std::thread::sleep(matchmaker::HEARTBEAT_INTERVAL);
        })
        .expect("could not start host heartbeat thread");
}

fn tick_heartbeat(
    handle: &tauri::AppHandle,
) -> Result<matchmaker::HeartbeatOutcome, matchmaker::MatchmakerError> {
    let dir = handle.path().app_config_dir().map_err(|error| {
        matchmaker::MatchmakerError::Network(format!(
            "could not resolve the app config directory: {error}"
        ))
    })?;
    matchmaker::heartbeat_from_env(&dir, &UreqTransport)
}

fn spawn_provision_listener(app: &tauri::App) {
    let state = app.state::<AgentState>();
    let occupancy = state.occupancy.clone();
    let capabilities = state.capabilities.clone();
    let hardware = state.hardware.clone();
    match provision_http::listen_config_from_env(|key| std::env::var(key).ok()) {
        Ok(None) => {}
        Ok(Some(config)) => {
            std::thread::Builder::new()
                .name("host-agent-provision".into())
                .spawn(move || {
                    let cache =
                        crate::image::resolve_image_cache_dir(|key| std::env::var(key).ok());
                    let gate = provision_http::ProvisionGate {
                        capabilities,
                        hardware,
                        sharing: None,
                        spare_disk_bytes: crate::disk_space::available_bytes(&cache),
                    };
                    if let Err(error) = provision_http::serve(
                        &config,
                        HostOs::current(),
                        occupancy.as_ref(),
                        Some(gate),
                    ) {
                        eprintln!("host-agent provision listener: {error}");
                    }
                })
                .expect("could not start host provision listener");
        }
        Err(error) => panic!("{error}"),
    }
}

pub fn run() {
    load_local_env();
    let hardware = hardware::detect();
    let os = HostOs::current();
    let capabilities = host_capabilities::probe_live(os, &hardware.gpus);
    let hypervisor = hypervisor::status_from_result(hypervisor::ensure_from_env(
        os,
        &capabilities,
        std::env::consts::ARCH,
        |key| std::env::var(key).ok(),
        &hypervisor::HttpBinaryFetcher,
    ));
    eprintln!("host-agent hypervisor: {}", hypervisor.log);
    let guest_image = match image::max_image_bytes(|key| std::env::var(key).ok()) {
        Ok(max_bytes) => image::status_from_result(image::ensure_from_env(
            os,
            &capabilities,
            |key| std::env::var(key).ok(),
            &image::HttpImageFetcher { max_bytes },
        )),
        Err(error) => image::status_from_result(Err(error)),
    };
    eprintln!("host-agent guest image: {}", guest_image.log);

    tauri::Builder::default()
        .manage(AgentState {
            hardware,
            capabilities,
            hypervisor,
            guest_image,
            occupancy: Arc::new(Occupancy::new()),
        })
        .setup(|app| {
            spawn_heartbeat_loop(app);
            spawn_provision_listener(app);
            tray::try_install(app);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if tray::is_installed() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_hardware_info,
            commands::get_host_capabilities,
            commands::get_hypervisor_status,
            commands::get_guest_image_status,
            commands::build_launch_config,
            commands::plan_gpu_bind,
            commands::evaluate_hosting_offers,
            commands::load_sharing_config,
            commands::save_sharing_config,
            commands::get_config_file_path,
            commands::load_host_registration,
            commands::register_declared_caps,
            commands::get_agent_status,
            commands::set_sharing_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the host agent");
}

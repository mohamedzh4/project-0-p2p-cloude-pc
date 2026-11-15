//! Detect the machine, persist local sharing caps, register those caps with
//! the Matchmaker from Rust, ping while sharing is on, and optionally join
//! self-hosted Headscale from Rust. Linux Standard provision starts QEMU+KVM
//! when the probe offers it and a verified disk is present. Windows Standard
//! provision starts QEMU with `-accel whpx` when the
//! probe offers it and the binary plus a verified disk are present. Linux Pro
//! provision applies vfio-pci bind when the probe offers Pro; Windows Pro
//! provision applies Hyper-V DDA/GPU-P when the probe offers Pro (window stays
//! plan-only). After a successful invoke or GPU apply, provision polls for a
//! reachable Kasm or Sunshine locator (mesh hostname; never an invented URL).
//! Tray status and immediate pause live in
//! Rust; the webview does not call the Matchmaker.

mod agent_status;
mod commands;
mod config;
mod disk_space;
mod eligibility;
mod env_file;
mod gpu;
mod gpu_bind;
mod guest_bootstrap;
mod hardware;
mod host_capabilities;
mod hypervisor;
mod identity;
mod image;
mod launch;
mod locator;
mod matchmaker;
mod mesh;
mod models;
mod provision;
mod provision_http;
mod qemu_whpx;
mod qemu_kvm;
mod qemu_vfio;
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
    /// Locate/install result for the OS QEMU binary. This status is
    /// locate/install only; guests are started by the provision listener.
    pub hypervisor: HypervisorStatus,
    /// Locate/fetch result for the signed guest image. InstanceStart is not
    /// a confirmed guest userspace.
    pub guest_image: GuestImageStatus,
    /// Set when provision HTTP 200 succeeds. Pause does not clear this; phase 12
    /// teardown will.
    pub occupancy: Arc<Occupancy>,
}

/// Reads this machine's hardware without opening a window, for diagnostics and
/// for the headless service this agent grows into.
pub fn detect_hardware() -> HardwareSnapshot {
    hardware::detect()
}

fn load_local_env() {
    // Packaged installs do not share `tauri dev`'s working directory. Load
    // `.env` from cwd, next to the binary, and the app-config dir. Process
    // environment still wins. HOST_AGENT_ENV_FILE, if set, must exist.
    match env_file::load_operator_env() {
        Ok(files) => {
            if files.is_empty() {
                eprintln!(
                    "host-agent env: no .env file found (cwd, executable dir, or {}); using process environment only",
                    env_file::CONFIG_DIR_NAME
                );
            } else {
                let listed = files
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                eprintln!("host-agent env: loaded {listed}");
            }
        }
        Err(error) => panic!("{error}"),
    }
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
    let hypervisor = state.hypervisor.clone();
    let guest_image = state.guest_image.clone();
    let os = HostOs::current();
    let config_dir = app.path().app_config_dir().ok();
    match provision_http::listen_config_from_env(|key| std::env::var(key).ok()) {
        Ok(None) => {}
        Ok(Some(config)) => {
            std::thread::Builder::new()
                .name("host-agent-provision".into())
                .spawn(move || {
                    let cache =
                        crate::image::resolve_image_cache_dir(|key| std::env::var(key).ok());
                    let sharing = config_dir
                        .as_ref()
                        .and_then(|dir| crate::config::load_for_hardware(dir, &hardware).ok());
                    let qemu_kvm = if os == HostOs::Linux && capabilities.standard.offered {
                        crate::qemu_kvm::resolve_binary(
                            std::env::consts::ARCH,
                            |key| std::env::var(key).ok(),
                        )
                        .ok()
                        .map(|binary| provision_http::QemuKvmBoot {
                            binary,
                            image_cache_dir: cache.clone(),
                            guest_image: guest_image.clone(),
                            arch: std::env::consts::ARCH.to_string(),
                            runtime: std::sync::Arc::new(
                                crate::qemu_kvm::ProcessQemuKvmRuntime,
                            ),
                        })
                    } else {
                        None
                    };
                    let qemu = if os == HostOs::Windows && capabilities.standard.offered {
                        Some(provision_http::QemuBoot {
                            binary: hypervisor
                                .path
                                .as_deref()
                                .map(std::path::PathBuf::from)
                                .unwrap_or_default(),
                            image_cache_dir: cache.clone(),
                            guest_image: guest_image.clone(),
                            arch: std::env::consts::ARCH.to_string(),
                            runtime: std::sync::Arc::new(crate::qemu_whpx::ProcessQemuRuntime),
                        })
                    } else {
                        None
                    };
                    let linux_qemu = if os == HostOs::Linux && capabilities.pro.offered {
                        crate::qemu_vfio::resolve_binary(
                            std::env::consts::ARCH,
                            |key| std::env::var(key).ok(),
                        )
                        .ok()
                        .map(|binary| provision_http::LinuxQemuBoot {
                            binary,
                            image_cache_dir: cache.clone(),
                            guest_image: guest_image.clone(),
                            arch: std::env::consts::ARCH.to_string(),
                            runtime: std::sync::Arc::new(
                                crate::qemu_vfio::ProcessLinuxQemuRuntime,
                            ),
                        })
                    } else {
                        None
                    };
                    let vfio = if os == HostOs::Linux && capabilities.pro.offered {
                        Some(provision_http::VfioBind {
                            sysfs: crate::gpu_bind::PciSysfs::live(),
                            ops: std::sync::Arc::new(crate::gpu_bind::SysfsBindOps),
                        })
                    } else {
                        None
                    };
                    let hyperv = if os == HostOs::Windows && capabilities.pro.offered {
                        Some(provision_http::HyperVAssign {
                            ops: std::sync::Arc::new(
                                crate::gpu_bind::ProcessHyperVGpuOps::default(),
                            ),
                            vm_name: Some(crate::gpu_bind::hyperv_vm_name_from_env(|key| {
                                std::env::var(key).ok()
                            })),
                        })
                    } else {
                        None
                    };
                    let gate = provision_http::ProvisionGate {
                        capabilities,
                        hardware,
                        sharing,
                        spare_disk_bytes: crate::disk_space::available_bytes(&cache),
                        qemu_kvm,
                        qemu,
                        vfio,
                        hyperv,
                        linux_qemu,
                        cleanup: provision_http::SessionCleanup::new(),
                        streamer: Some(crate::locator::StreamerWait::production()),
                    };
                    if let Err(error) =
                        provision_http::serve(&config, os, occupancy.as_ref(), Some(gate))
                    {
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
        &hypervisor::ProcessQemuInstaller,
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

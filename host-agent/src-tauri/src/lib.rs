//! Host Agent core.
//!
//! Phase 1 scope: detect the machine's hardware, let the host cap what they
//! share, and persist that choice locally. Networking, matchmaking and VM
//! provisioning are deliberately absent — the modules here are split so those
//! can be added beside them without touching detection or storage.

mod commands;
mod config;
mod gpu;
mod hardware;
mod models;

#[cfg(test)]
mod test_support;

pub use models::HardwareSnapshot;

/// Long-lived state shared by every command.
pub struct AgentState {
    /// Probed once at startup: the machine's hardware does not change while the
    /// agent runs, and re-probing per request would waste the resources the
    /// agent exists to lend out.
    pub hardware: HardwareSnapshot,
}

/// Reads this machine's hardware without opening a window, for diagnostics and
/// for the headless service this agent grows into.
pub fn detect_hardware() -> HardwareSnapshot {
    hardware::detect()
}

pub fn run() {
    tauri::Builder::default()
        .manage(AgentState {
            hardware: hardware::detect(),
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_hardware_info,
            commands::load_sharing_config,
            commands::save_sharing_config,
            commands::get_config_file_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the host agent");
}

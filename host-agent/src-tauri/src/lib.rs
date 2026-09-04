//! Detect the machine, persist local sharing caps, register those caps with
//! the Matchmaker from Rust, and ping while sharing is on. Provisioning and
//! tray mode are still out of scope.

mod commands;
mod config;
mod gpu;
mod hardware;
mod identity;
mod matchmaker;
mod models;

#[cfg(test)]
mod test_support;

pub use models::HardwareSnapshot;

use tauri::Manager;

use crate::matchmaker::UreqTransport;

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

pub fn run() {
    load_local_env();

    tauri::Builder::default()
        .manage(AgentState {
            hardware: hardware::detect(),
        })
        .setup(|app| {
            spawn_heartbeat_loop(app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_hardware_info,
            commands::load_sharing_config,
            commands::save_sharing_config,
            commands::get_config_file_path,
            commands::load_host_registration,
            commands::register_declared_caps,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the host agent");
}

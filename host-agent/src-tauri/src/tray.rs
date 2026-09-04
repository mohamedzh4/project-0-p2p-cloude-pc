//! System tray built in Rust. The webview never calls tray or menu APIs, so
//! capabilities stay without `core:tray` / `core:menu`.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::agent_status::AgentStatus;
use crate::config;
use crate::AgentState;

pub const TRAY_ID: &str = "host-status";

static INSTALLED: AtomicBool = AtomicBool::new(false);

pub fn is_installed() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

/// Best-effort: a missing icon or platform tray failure must not kill the agent.
pub fn try_install(app: &tauri::App) {
    match install(app) {
        Ok(()) => INSTALLED.store(true, Ordering::SeqCst),
        Err(error) => eprintln!("host-agent tray: {error}"),
    }
}

fn install(app: &tauri::App) -> Result<(), String> {
    let Some(icon) = app.default_window_icon().cloned() else {
        return Err("default window icon required for the tray".into());
    };

    let pause = MenuItem::with_id(app, "pause", "Pause sharing", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let resume = MenuItem::with_id(app, "resume", "Resume sharing", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let show = MenuItem::with_id(app, "show", "Open Host Agent", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let quit = MenuItem::with_id(app, "quit", "Quit Host Agent", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let menu = Menu::with_items(app, &[&pause, &resume, &show, &quit])
        .map_err(|error| error.to_string())?;

    let tooltip = current_status(app.handle()).map(|status| status.tooltip());

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip(tooltip.unwrap_or_else(|| "Host Agent".into()))
        .on_menu_event(|app, event| match event.id().as_ref() {
            "pause" => {
                let _ = persist_sharing(app, false);
                refresh(app);
            }
            "resume" => {
                let _ = persist_sharing(app, true);
                refresh(app);
            }
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|error| error.to_string())?;

    Ok(())
}

pub fn refresh(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let tooltip = current_status(app)
        .map(|status| status.tooltip())
        .unwrap_or_else(|| "Host Agent".into());
    let _ = tray.set_tooltip(Some(tooltip));
}

fn current_status(app: &AppHandle) -> Option<AgentStatus> {
    let state = app.try_state::<AgentState>()?;
    let dir = app.path().app_config_dir().ok()?;
    let config = config::load_for_hardware(&dir, &state.hardware).ok()?;
    Some(AgentStatus::from_flags(
        config.sharing_enabled,
        state.occupancy.is_in_session(),
        None,
    ))
}

fn persist_sharing(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let state = app.state::<AgentState>();
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("could not resolve the app config directory: {error}"))?;
    config::set_sharing_enabled(&dir, &state.hardware, enabled)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

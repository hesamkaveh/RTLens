//! Menu bar presence.

use crate::{trigger, window};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Runtime};
use tauri_plugin_autostart::ManagerExt;

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let capture = MenuItem::with_id(app, "capture", "Capture Now", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart =
        CheckMenuItem::with_id(app, "autostart", "Launch at Login", true, autostart_on, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit RTLens", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &capture,
            &PredefinedMenuItem::separator(app)?,
            &settings,
            &autostart,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    // Embedded rather than read from disk so the tray works from any working directory.
    // The @2x asset is supplied so it stays crisp on Retina; macOS scales it to 22pt.
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray@2x.png"))?;

    TrayIconBuilder::with_id("main")
        .icon(icon)
        // A template icon is recoloured by macOS to match a light or dark menu bar.
        .icon_as_template(true)
        .tooltip("RTLens")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "capture" => trigger(app),
            "settings" => {
                if let Err(e) = window::open_settings(app) {
                    tracing::error!("could not open settings: {e}");
                }
            }
            "autostart" => {
                let manager = app.autolaunch();
                let enabled = manager.is_enabled().unwrap_or(false);
                let result = if enabled { manager.disable() } else { manager.enable() };
                if let Err(e) = result {
                    tracing::error!("could not toggle autostart: {e}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

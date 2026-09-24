//! HUD window lifecycle.
//!
//! The window is built once at startup and kept alive, hidden, for the life of the
//! process. Constructing a webview costs tens of milliseconds and would dominate the
//! latency budget if it happened on every trigger.

use tauri::{AppHandle, LogicalSize, Manager, PhysicalPosition, Runtime, WebviewWindow};

#[cfg(target_os = "macos")]
use std::{path::Path, process::Command, time::Duration};

pub const HUD: &str = "hud";
pub const SETTINGS: &str = "settings";

/// Gap between the cursor and the HUD, and between the HUD and a screen edge.
const MARGIN: f64 = 16.0;

pub fn hud<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window(HUD)
}

/// Apply the platform's background blur. Failure is non-fatal: a solid background is a
/// worse-looking HUD, not a broken one.
pub fn apply_effects<R: Runtime>(window: &WebviewWindow<R>) {
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
        if let Err(e) = apply_vibrancy(
            window,
            NSVisualEffectMaterial::HudWindow,
            Some(NSVisualEffectState::Active),
            Some(16.0),
        ) {
            tracing::warn!("vibrancy unavailable: {e}");
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Err(e) = window_vibrancy::apply_acrylic(window, Some((0, 0, 0, 90))) {
            tracing::warn!("acrylic unavailable: {e}");
        }
    }
    #[cfg(target_os = "linux")]
    {
        // Blur on Linux is the compositor's decision; the HUD falls back to opacity.
        let _ = window;
    }
}

/// Resize to the measured content height, place near the cursor, and reveal.
///
/// Called once the frontend has rendered and measured itself, so the window never appears
/// at the wrong size and then jumps.
pub fn present<R: Runtime>(app: &AppHandle<R>, content_height: f64) {
    let Some(window) = hud(app) else { return };
    let already_visible = window.is_visible().unwrap_or(false);

    let width = window
        .outer_size()
        .ok()
        .and_then(|s| window.scale_factor().ok().map(|f| s.width as f64 / f))
        .unwrap_or(720.0);
    let height = content_height.clamp(120.0, 720.0);

    if let Err(e) = window.set_size(LogicalSize::new(width, height)) {
        tracing::warn!("could not resize HUD: {e}");
    }

    if !already_visible {
        if let Err(e) = position_near_cursor(app, &window) {
            tracing::warn!("could not position HUD: {e}");
        }
    } else if let Err(e) = clamp_to_screen(app, &window) {
        tracing::warn!("could not clamp HUD to screen: {e}");
    }

    let _ = window.show();
    // Focus is taken deliberately. The selection was captured before the window appeared,
    // so there is nothing left to steal, and without focus the Escape and Enter keys —
    // the only way to dismiss or copy — would never reach the webview.
    let _ = window.set_focus();
}

fn clamp_to_screen<R: Runtime>(app: &AppHandle<R>, window: &WebviewWindow<R>) -> tauri::Result<()> {
    let pos = window.outer_position()?;
    let size = window.outer_size()?;
    let monitor = app.monitor_from_point(pos.x as f64, pos.y as f64)?.or(app.primary_monitor()?);
    let Some(monitor) = monitor else { return Ok(()) };

    let area = monitor.size();
    let origin = monitor.position();
    let scale = monitor.scale_factor();
    let margin = MARGIN * scale;

    let max_x = (origin.x + area.width as i32) as f64 - size.width as f64 - margin;
    let max_y = (origin.y + area.height as i32) as f64 - size.height as f64 - margin;
    let x = (pos.x as f64).min(max_x).max(origin.x as f64 + margin);
    let y = (pos.y as f64).min(max_y).max(origin.y as f64 + margin);

    if (x - pos.x as f64).abs() > 1.0 || (y - pos.y as f64).abs() > 1.0 {
        window.set_position(PhysicalPosition::new(x, y))?;
    }
    Ok(())
}

fn position_near_cursor<R: Runtime>(app: &AppHandle<R>, window: &WebviewWindow<R>) -> tauri::Result<()> {
    let cursor = app.cursor_position()?;
    let monitor = app.monitor_from_point(cursor.x, cursor.y)?.or(app.primary_monitor()?);
    let Some(monitor) = monitor else { return Ok(()) };

    let size = window.outer_size()?;
    let area = monitor.size();
    let origin = monitor.position();
    let scale = monitor.scale_factor();
    let margin = MARGIN * scale;

    // Offer the HUD below-right of the cursor, then pull it back inside the screen.
    let mut x = cursor.x + margin;
    let mut y = cursor.y + margin;
    let max_x = (origin.x + area.width as i32) as f64 - size.width as f64 - margin;
    let max_y = (origin.y + area.height as i32) as f64 - size.height as f64 - margin;
    x = x.min(max_x).max(origin.x as f64 + margin);
    y = y.min(max_y).max(origin.y as f64 + margin);

    window.set_position(PhysicalPosition::new(x, y))
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = hud(app) {
        let _ = window.hide();
    }
}

pub fn is_visible<R: Runtime>(app: &AppHandle<R>) -> bool {
    hud(app).and_then(|w| w.is_visible().ok()).unwrap_or(false)
}

/// Open (or focus) the settings window, creating it on first use.
pub fn open_settings<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    let aerospace_workspace = active_aerospace_workspace();

    if let Some(window) = app.get_webview_window(SETTINGS) {
        window.show()?;
        #[cfg(target_os = "macos")]
        if let Some(target) = aerospace_workspace {
            move_to_aerospace_workspace(&window, target);
        }
        window.set_focus()?;
        return Ok(());
    }
    let window =
        tauri::WebviewWindowBuilder::new(app, SETTINGS, tauri::WebviewUrl::App("settings.html".into()))
            .title("RTLens Settings")
            .inner_size(520.0, 620.0)
            .resizable(true)
            .build()?;

    #[cfg(target_os = "macos")]
    if let Some(target) = aerospace_workspace {
        move_to_aerospace_workspace(&window, target);
    }
    window.set_focus()
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct AeroSpaceWorkspace {
    executable: &'static str,
    name: String,
}

#[cfg(target_os = "macos")]
fn active_aerospace_workspace() -> Option<AeroSpaceWorkspace> {
    const CANDIDATES: [&str; 3] = ["/opt/homebrew/bin/aerospace", "/usr/local/bin/aerospace", "aerospace"];

    for executable in CANDIDATES {
        if executable != "aerospace" && !Path::new(executable).is_file() {
            continue;
        }
        let Ok(output) = Command::new(executable)
            .args(["list-workspaces", "--focused", "--format", "%{workspace}"])
            .output()
        else {
            continue;
        };
        if output.status.success() {
            if let Some(name) = parse_workspace(&output.stdout) {
                tracing::debug!("AeroSpace focused workspace: {name}");
                return Some(AeroSpaceWorkspace { executable, name });
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn parse_workspace(stdout: &[u8]) -> Option<String> {
    let value = std::str::from_utf8(stdout).ok()?.trim();
    (!value.is_empty() && !value.contains(['\n', '\r'])).then(|| value.to_owned())
}

#[cfg(target_os = "macos")]
fn move_to_aerospace_workspace<R: Runtime>(window: &WebviewWindow<R>, target: AeroSpaceWorkspace) {
    let Ok(pointer) = window.ns_window() else {
        return;
    };
    // SAFETY: Tauri returns this pointer for the live NSWindow owned by `window`, and
    // this method is called on the main thread while that handle is still alive.
    let window_id = unsafe { (&*pointer.cast::<objc2_app_kit::NSWindow>()).windowNumber() }.to_string();

    std::thread::spawn(move || {
        for attempt in 0..10 {
            let status = Command::new(target.executable)
                .args([
                    "move-node-to-workspace",
                    "--focus-follows-window",
                    "--window-id",
                    &window_id,
                    &target.name,
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if status.is_ok_and(|status| status.success()) {
                tracing::debug!("moved settings window {window_id} to AeroSpace workspace {}", target.name);
                return;
            }
            if attempt < 9 {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        tracing::warn!("AeroSpace did not accept settings window {window_id}");
    });
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::parse_workspace;

    #[test]
    fn parses_one_focused_aerospace_workspace() {
        assert_eq!(parse_workspace(b"  dev \n"), Some("dev".to_string()));
    }

    #[test]
    fn rejects_missing_or_ambiguous_aerospace_workspace() {
        assert_eq!(parse_workspace(b"\n"), None);
        assert_eq!(parse_workspace(b"one\ntwo\n"), None);
        assert_eq!(parse_workspace(&[0xff]), None);
    }
}

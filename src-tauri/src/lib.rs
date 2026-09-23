//! RTLens application shell: trigger, capture, analyse, present.

mod antigravity;
mod capture;
mod config;
mod hotkey;
mod tray;
mod window;

use capture::{Capturer, DEFAULT_BUDGET};
use config::Settings;
use rtlens_core::{Doc, Options};
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime, WindowEvent};
use tauri_plugin_global_shortcut::ShortcutState;

/// Shown if the frontend never reports its measured height, so a rendering bug cannot
/// leave the user with a hotkey that silently does nothing.
const PRESENT_FALLBACK: Duration = Duration::from_millis(300);
const FALLBACK_HEIGHT: f64 = 320.0;

pub struct RtState {
    settings: Mutex<Settings>,
    /// Raw captured text, retained so engine toggles can re-run without re-capturing.
    last_raw: Mutex<String>,
    last_doc: Mutex<Option<Doc>>,
    capturer: Box<dyn Capturer>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocPayload {
    doc: Doc,
    settings: Settings,
    source_app: Option<String>,
    /// False when the OS is withholding the permission needed to capture a selection.
    can_capture: bool,
}

/// Capture, analyse and show. Safe to call from any thread.
pub fn trigger<R: Runtime>(app: &AppHandle<R>) {
    // A second press while the HUD is up means "put it away".
    if window::is_visible(app) {
        window::hide(app);
        return;
    }

    let app = app.clone();
    // Capture blocks for as long as the target application takes to answer, so it must
    // never run on the UI thread.
    std::thread::spawn(move || {
        let started = Instant::now();
        let state = app.state::<RtState>();
        let settings = state.settings.lock().expect("settings lock").clone();

        let captured = state.capturer.capture(DEFAULT_BUDGET, settings.clipboard_fallback);
        let can_capture = state.capturer.can_synthesize();
        let doc = rtlens_core::process(&captured.text, &settings.engine, captured.source);

        *state.last_raw.lock().expect("raw lock") = captured.text;
        *state.last_doc.lock().expect("doc lock") = Some(doc.clone());

        let payload = DocPayload { doc, settings, source_app: captured.app, can_capture };
        if let Err(e) = app.emit_to(window::HUD, "rtlens://doc", payload) {
            tracing::error!("could not deliver document to the HUD: {e}");
        }
        tracing::debug!("capture to emit: {:?}", started.elapsed());

        let fallback = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(PRESENT_FALLBACK);
            if !window::is_visible(&fallback) {
                tracing::warn!("frontend did not report a height; presenting at default size");
                window::present(&fallback, FALLBACK_HEIGHT);
            }
        });
    });
}

#[cfg(target_os = "macos")]
fn should_open_settings_on_reopen(has_visible_windows: bool) -> bool {
    !has_visible_windows
}

// ---------------------------------------------------------------------------- commands

/// The frontend has rendered and measured itself; reveal the window at that height.
#[tauri::command]
fn hud_ready<R: Runtime>(app: AppHandle<R>, height: f64) {
    window::present(&app, height);
}

#[tauri::command]
fn hide_hud<R: Runtime>(app: AppHandle<R>) {
    window::hide(&app);
}

/// Tell every window that settings changed.
///
/// Without this the HUD and the settings window each hold their own copy, so changing a
/// setting would not affect a HUD that is already on screen.
fn broadcast_settings<R: Runtime>(app: &AppHandle<R>, settings: &Settings) {
    if let Err(e) = app.emit("rtlens://settings", settings) {
        tracing::warn!("could not broadcast settings: {e}");
    }
}

#[tauri::command]
fn get_settings<R: Runtime>(app: AppHandle<R>) -> Settings {
    app.state::<RtState>().settings.lock().expect("settings lock").clone()
}

#[tauri::command]
fn save_settings<R: Runtime>(app: AppHandle<R>, settings: Settings) -> Result<Settings, String> {
    let previous = {
        let state = app.state::<RtState>();
        let current = state.settings.lock().expect("settings lock").clone();
        current.shortcut
    };

    if previous != settings.shortcut {
        hotkey::rebind(&app, Some(&previous), &settings.shortcut)?;
    }

    config::save(&app, &settings)?;
    app.state::<antigravity::Antigravity>().set_enabled(settings.antigravity_rtl);
    *app.state::<RtState>().settings.lock().expect("settings lock") = settings.clone();
    broadcast_settings(&app, &settings);
    Ok(settings)
}

/// Re-run the engine over the text already captured, with different toggles.
#[tauri::command]
fn set_engine<R: Runtime>(app: AppHandle<R>, engine: Options) -> Result<Doc, String> {
    let state = app.state::<RtState>();
    let raw = state.last_raw.lock().expect("raw lock").clone();
    let source = state
        .last_doc
        .lock()
        .expect("doc lock")
        .as_ref()
        .map(|d| d.source)
        .unwrap_or(rtlens_core::CaptureSource::Empty);

    let doc = rtlens_core::process(&raw, &engine, source);
    *state.last_doc.lock().expect("doc lock") = Some(doc.clone());

    let mut settings = state.settings.lock().expect("settings lock");
    settings.engine = engine;
    let snapshot = settings.clone();
    drop(settings);
    config::save(&app, &snapshot)?;
    broadcast_settings(&app, &snapshot);

    Ok(doc)
}

/// Update the dismiss_on_blur setting, persist it, and broadcast to all windows.
#[tauri::command]
fn set_dismiss_on_blur<R: Runtime>(app: AppHandle<R>, dismiss_on_blur: bool) -> Result<bool, String> {
    let state = app.state::<RtState>();
    let mut settings = state.settings.lock().expect("settings lock");
    settings.dismiss_on_blur = dismiss_on_blur;
    let snapshot = settings.clone();
    drop(settings);
    config::save(&app, &snapshot)?;
    broadcast_settings(&app, &snapshot);
    Ok(snapshot.dismiss_on_blur)
}

#[tauri::command]
fn antigravity_status<R: Runtime>(app: AppHandle<R>) -> antigravity::Status {
    app.state::<antigravity::Antigravity>().status()
}

#[tauri::command]
fn accessibility_status() -> bool {
    #[cfg(target_os = "macos")]
    {
        capture::macos::accessibility_trusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Send the user to the exact pane they need, rather than describing where it is.
#[tauri::command]
fn open_accessibility_settings<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        capture::macos::request_accessibility_permission();
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_url(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
                None::<&str>,
            )
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(())
    }
}

#[tauri::command]
fn request_accessibility_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        capture::macos::request_accessibility_permission()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

// ------------------------------------------------------------------------------- setup

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "rtlens=info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    // Fire on press only; acting on release too would trigger twice.
                    if event.state() == ShortcutState::Pressed {
                        trigger(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            hud_ready,
            hide_hud,
            get_settings,
            save_settings,
            set_engine,
            set_dismiss_on_blur,
            antigravity_status,
            accessibility_status,
            request_accessibility_permission,
            open_accessibility_settings,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // No dock icon, and dismissing the HUD hands focus back to the app the user
            // was actually working in.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let settings = config::load(&handle);
            app.manage(antigravity::Antigravity::start(settings.antigravity_rtl));
            app.manage(RtState {
                last_raw: Mutex::new(String::new()),
                last_doc: Mutex::new(None),
                capturer: capture::platform(),
                settings: Mutex::new(settings.clone()),
            });

            match hotkey::rebind(&handle, None, &settings.shortcut) {
                Ok(()) => tracing::info!("trigger bound to {}", settings.shortcut),
                Err(e) => {
                    tracing::error!("{e}");
                    // Fall back to the default so the app is never left with no trigger.
                    if settings.shortcut != config::default_shortcut() {
                        let _ = hotkey::rebind(&handle, None, &config::default_shortcut());
                    }
                }
            }

            #[cfg(target_os = "macos")]
            tracing::info!(
                "accessibility access: {}",
                if capture::macos::accessibility_trusted() { "granted" } else { "NOT granted" }
            );

            if let Some(hud) = window::hud(&handle) {
                window::apply_effects(&hud);
                // Debug builds only: RTLENS_DEVTOOLS=1 shows the HUD at startup with the
                // inspector attached, which is the only practical way to debug a window
                // that is normally hidden.
                #[cfg(debug_assertions)]
                if std::env::var("RTLENS_DEVTOOLS").is_ok() {
                    let _ = hud.show();
                    let _ = hud.set_focus();
                    hud.open_devtools();
                }
                let dismiss_handle = handle.clone();
                hud.on_window_event(move |event| {
                    if let WindowEvent::Focused(false) = event {
                        let dismiss = dismiss_handle
                            .state::<RtState>()
                            .settings
                            .lock()
                            .expect("settings lock")
                            .dismiss_on_blur;
                        if dismiss {
                            window::hide(&dismiss_handle);
                        }
                    }
                });
            }

            tray::build(&handle)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the HUD should put it away, not tear down the app: the window is
            // built once and reused for every capture.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == window::HUD {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building RTLens")
        .run(|app, event| match event {
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { has_visible_windows, .. }
                if should_open_settings_on_reopen(has_visible_windows) =>
            {
                if let Err(error) = window::open_settings(app) {
                    tracing::error!("failed to open settings after Dock activation: {error}");
                }
            }
            _ => {}
        });
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::should_open_settings_on_reopen;

    #[test]
    fn dock_reopen_opens_settings_when_all_windows_are_hidden() {
        assert!(should_open_settings_on_reopen(false));
        assert!(!should_open_settings_on_reopen(true));
    }
}

//! Global trigger registration.
//!
//! Uses the OS's own hotkey registry (Carbon `RegisterEventHotKey` on macOS) rather than
//! an event tap. That distinction matters: a tap would see every keystroke the user types
//! and require Input Monitoring permission, and is the mechanism that breaks tmux and
//! terminal bindings. A registered hotkey can only ever fire for its own combination.
//!
//! The module is shaped so a double-tap detector can be added later as a second trigger
//! source without changing how the rest of the app is notified.

use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

/// Swap the registered trigger, rolling back if the new one cannot be bound.
///
/// A shortcut can fail because it is malformed or already owned by another application.
/// Leaving the user with no working trigger — and a settings pane that claims otherwise —
/// would be worse than refusing the change.
pub fn rebind<R: Runtime>(app: &AppHandle<R>, previous: Option<&str>, next: &str) -> Result<(), String> {
    let parsed: Shortcut = next.parse().map_err(|e| format!("'{next}' is not a valid shortcut: {e}"))?;

    if let Some(previous) = previous {
        if let Ok(old) = previous.parse::<Shortcut>() {
            let _ = app.global_shortcut().unregister(old);
        }
    }

    match app.global_shortcut().register(parsed) {
        Ok(()) => Ok(()),
        Err(e) => {
            if let Some(previous) = previous {
                if let Ok(old) = previous.parse::<Shortcut>() {
                    let _ = app.global_shortcut().register(old);
                }
            }
            Err(format!("could not register '{next}': {e}"))
        }
    }
}

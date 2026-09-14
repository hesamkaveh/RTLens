//! Persisted settings.

use rtlens_core::Options;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

pub const STORE_FILE: &str = "settings.json";
const KEY: &str = "settings";

/// The default trigger.
///
/// Deliberately not ⌘⇧R: a global shortcut outranks application shortcuts, so binding
/// that would disable hard-reload in every browser for as long as RTLens is running.
pub fn default_shortcut() -> String {
    "Control+Alt+R".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub shortcut: String,
    pub engine: Options,
    /// Hide the HUD when it loses focus.
    pub dismiss_on_blur: bool,
    /// When no selection is detected, show the current clipboard instead of an empty state.
    pub clipboard_fallback: bool,
    pub font_size: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shortcut: default_shortcut(),
            engine: Options::default(),
            dismiss_on_blur: true,
            clipboard_fallback: true,
            font_size: 14,
        }
    }
}

/// Read settings, falling back to defaults if the file is missing or malformed.
///
/// A corrupt settings file must never stop the app from starting; the user would have no
/// way to fix it from a UI that will not open.
pub fn load<R: Runtime>(app: &AppHandle<R>) -> Settings {
    let Ok(store) = app.store(STORE_FILE) else {
        tracing::warn!("settings store unavailable; using defaults");
        return Settings::default();
    };
    store
        .get(KEY)
        .and_then(|value| match serde_json::from_value(value) {
            Ok(settings) => Some(settings),
            Err(e) => {
                tracing::warn!("ignoring malformed settings: {e}");
                None
            }
        })
        .unwrap_or_default()
}

pub fn save<R: Runtime>(app: &AppHandle<R>, settings: &Settings) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.set(KEY, serde_json::to_value(settings).map_err(|e| e.to_string())?);
    store.save().map_err(|e| e.to_string())
}

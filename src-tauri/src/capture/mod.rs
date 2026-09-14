//! Getting the user's current selection out of an application that will not tell us
//! what it is.
//!
//! GPU-accelerated terminals (Ghostty, Alacritty, WezTerm) render text themselves and do
//! not expose a selection through the platform accessibility APIs, so reading
//! `AXSelectedText` returns nothing useful for exactly the applications RTLens exists to
//! serve. The reliable path is to ask the app to copy, then put the clipboard back.
//!
//! Every platform implements [`Capturer`]; only the macOS one has been run on real
//! hardware.

use rtlens_core::CaptureSource;
use std::time::Duration;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub mod fallback;

/// A capture attempt's result.
#[derive(Debug, Clone)]
pub struct Captured {
    pub text: String,
    pub source: CaptureSource,
    /// Bundle id / process name of the app the selection came from, when known.
    pub app: Option<String>,
}

impl Captured {
    // Used only by the non-macOS capturers.
    #[allow(dead_code)]
    pub fn empty() -> Self {
        Self { text: String::new(), source: CaptureSource::Empty, app: None }
    }
}

/// How long to wait for the target application to answer a synthetic copy.
///
/// Terminals typically respond in 15-60 ms. The ceiling is generous because a busy
/// application is slow rather than broken, and the cost of giving up early is showing the
/// user stale clipboard content.
pub const DEFAULT_BUDGET: Duration = Duration::from_millis(350);

/// Platform selection capture.
pub trait Capturer: Send + Sync {
    /// Capture the current selection, restoring the clipboard before returning.
    fn capture(&self, budget: Duration, clipboard_fallback: bool) -> Captured;

    /// Whether the OS currently permits synthesising input. When false the caller should
    /// route the user to onboarding instead of silently degrading.
    fn can_synthesize(&self) -> bool;
}

/// The capturer for the platform this build targets.
pub fn platform() -> Box<dyn Capturer> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacCapturer)
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        Box::new(fallback::FallbackCapturer)
    }
}

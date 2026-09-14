//! Windows and Linux selection capture.
//!
//! UNVERIFIED: written against the documented behaviour of `arboard` and `enigo` but never
//! compiled or run — this build was developed on macOS only. Treat as a starting point.
//!
//! Two deliberate differences from the macOS path:
//!
//! * **`Ctrl+Shift+C`, never `Ctrl+C`.** In Windows Terminal and most Linux terminals,
//!   `Ctrl+C` with no active selection is passed through to the foreground process as
//!   SIGINT. Synthesising it would kill the user's running build. `Ctrl+Shift+C` is the
//!   copy binding in Windows Terminal, GNOME Terminal, Konsole and Alacritty alike.
//! * **X11 primary selection first.** Under X11 a selection is already readable without
//!   synthesising anything, so the whole risky dance is skipped when it is available.
//!
//! Clipboard backup here covers text and images only. The macOS implementation preserves
//! every pasteboard type; matching that on Windows needs raw `EnumClipboardFormats` work
//! that should not be written without hardware to test it on.

use super::{Captured, Capturer};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use rtlens_core::CaptureSource;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(5);
const SETTLE: Duration = Duration::from_millis(30);

enum Backup {
    Text(String),
    Image(arboard::ImageData<'static>),
    None,
}

fn back_up(clipboard: &mut arboard::Clipboard) -> Backup {
    if let Ok(text) = clipboard.get_text() {
        return Backup::Text(text);
    }
    if let Ok(image) = clipboard.get_image() {
        return Backup::Image(image.to_owned_img());
    }
    Backup::None
}

fn put_back(backup: Backup) {
    let Ok(mut clipboard) = arboard::Clipboard::new() else { return };
    let result = match backup {
        Backup::Text(t) => clipboard.set_text(t),
        Backup::Image(i) => clipboard.set_image(i),
        Backup::None => return,
    };
    if let Err(e) = result {
        tracing::warn!("failed to restore clipboard: {e}");
    }
}

/// Read the X11 primary selection, which holds the current selection with no synthetic
/// input at all. Returns `None` under Wayland or when nothing is selected.
#[cfg(target_os = "linux")]
fn primary_selection() -> Option<String> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    use arboard::LinuxClipboardKind;
    let text = clipboard.get().clipboard(LinuxClipboardKind::Primary).text().ok()?;
    (!text.trim().is_empty()).then_some(text)
}

#[cfg(not(target_os = "linux"))]
fn primary_selection() -> Option<String> {
    None
}

fn send_copy() -> bool {
    let Ok(mut enigo) = Enigo::new(&Settings::default()) else { return false };
    let sequence = [
        (Key::Control, Direction::Press),
        (Key::Shift, Direction::Press),
        (Key::Unicode('c'), Direction::Click),
        (Key::Shift, Direction::Release),
        (Key::Control, Direction::Release),
    ];
    for (key, direction) in sequence {
        if enigo.key(key, direction).is_err() {
            return false;
        }
    }
    true
}

pub struct FallbackCapturer;

impl Capturer for FallbackCapturer {
    fn capture(&self, budget: Duration, clipboard_fallback: bool) -> Captured {
        // Zero-risk path: no synthetic input, no clipboard mutation, nothing to restore.
        if let Some(text) = primary_selection() {
            return Captured { text, source: CaptureSource::Selection, app: None };
        }

        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            return Captured::empty();
        };
        let before_text = clipboard.get_text().unwrap_or_default();
        let backup = back_up(&mut clipboard);

        if !send_copy() {
            return Captured { text: before_text, source: CaptureSource::ClipboardFallback, app: None };
        }

        // There is no clipboard sequence number available through arboard, so change is
        // detected by content. Re-copying identical text therefore reads as "no selection",
        // which is harmless: the text shown is the same either way.
        let deadline = Instant::now() + budget;
        let mut captured = None;
        while Instant::now() < deadline {
            if let Ok(text) = clipboard.get_text() {
                if text != before_text && !text.is_empty() {
                    captured = Some(text);
                    break;
                }
            }
            std::thread::sleep(POLL_INTERVAL);
        }

        match captured {
            Some(text) => {
                std::thread::spawn(move || {
                    std::thread::sleep(SETTLE);
                    put_back(backup);
                });
                Captured { text, source: CaptureSource::Selection, app: None }
            }
            None => Captured {
                source: if before_text.is_empty() || !clipboard_fallback {
                    CaptureSource::Empty
                } else {
                    CaptureSource::ClipboardFallback
                },
                text: if clipboard_fallback { before_text } else { String::new() },
                app: None,
            },
        }
    }

    fn can_synthesize(&self) -> bool {
        // X11 needs no synthesis; Wayland generally cannot synthesise without ydotool.
        primary_selection().is_some() || cfg!(target_os = "windows")
    }
}

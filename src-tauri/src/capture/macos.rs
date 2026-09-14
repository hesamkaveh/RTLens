//! macOS selection capture: full-fidelity pasteboard swap around a synthetic ⌘C.
//!
//! The pasteboard is backed up by *type*, not just as text, so an image, RTF snippet or
//! file URL the user had copied survives the round trip. Anything less would mean RTLens
//! silently destroying the user's clipboard every time it runs.

use super::{Captured, Capturer};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString, NSWorkspace};
use objc2_foundation::{NSArray, NSData, NSString};
use rtlens_core::CaptureSource;
use std::time::{Duration, Instant};

/// `kVK_ANSI_C` from Carbon's `Events.h`. Virtual key codes are positional, not
/// character-based, so this is correct regardless of the user's keyboard layout.
const KEY_C: u16 = 0x08;

/// Polling interval while waiting for the target app to write to the pasteboard.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Grace period before restoring, so a slow app finishes writing every type it intends to.
const SETTLE: Duration = Duration::from_millis(30);

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

/// True when this process may synthesise keyboard events (System Settings → Privacy &
/// Security → Accessibility).
pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Every type currently on the pasteboard, with its raw bytes.
struct Snapshot {
    change_count: isize,
    items: Vec<(String, Vec<u8>)>,
}

fn pasteboard() -> objc2::rc::Retained<NSPasteboard> {
    NSPasteboard::generalPasteboard()
}

fn change_count() -> isize {
    pasteboard().changeCount()
}

fn snapshot() -> Snapshot {
    let pb = pasteboard();
    let change_count = pb.changeCount();
    let mut items = Vec::new();
    if let Some(types) = pb.types() {
        for ty in types.iter() {
            if let Some(data) = pb.dataForType(&ty) {
                items.push((ty.to_string(), data.to_vec()));
            }
        }
    }
    Snapshot { change_count, items }
}

/// Put the user's clipboard back exactly as it was.
///
/// Skipped when the pasteboard moved on again after our copy: that means another app (or
/// the user) wrote to it in the meantime, and clobbering that would be worse than leaving
/// our transient copy in place.
fn restore(snap: &Snapshot, expected: isize) {
    if change_count() != expected {
        tracing::debug!("pasteboard changed after capture; skipping restore");
        return;
    }
    if snap.items.is_empty() {
        return;
    }
    let pb = pasteboard();
    let types: Vec<objc2::rc::Retained<NSString>> =
        snap.items.iter().map(|(ty, _)| NSString::from_str(ty)).collect();
    let refs: Vec<&NSString> = types.iter().map(|t| t.as_ref()).collect();
    let array = NSArray::from_slice(&refs);
    unsafe {
        pb.declareTypes_owner(&array, None);
        for ((ty, bytes), ns_ty) in snap.items.iter().zip(types.iter()) {
            let data = NSData::with_bytes(bytes);
            if !pb.setData_forType(Some(&data), ns_ty) {
                tracing::warn!("failed to restore pasteboard type {ty}");
            }
        }
    }
}

fn read_text() -> String {
    let pb = pasteboard();
    unsafe { pb.stringForType(NSPasteboardTypeString) }.map(|s| s.to_string()).unwrap_or_default()
}

/// Bundle identifier of the frontmost application, used for per-app behaviour and logging.
pub fn frontmost_app() -> Option<String> {
    let workspace = NSWorkspace::sharedWorkspace();
    let app = workspace.frontmostApplication()?;
    app.bundleIdentifier().map(|s| s.to_string())
}

/// Synthesise ⌘C into whatever is frontmost.
///
/// On macOS this is unambiguously "copy" in every terminal, so unlike Ctrl+C on Windows
/// and Linux there is no risk of the keystroke being interpreted as an interrupt.
fn send_copy() -> bool {
    let Ok(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) else {
        tracing::error!("could not create a CGEventSource");
        return false;
    };
    let (Ok(down), Ok(up)) = (
        CGEvent::new_keyboard_event(source.clone(), KEY_C, true),
        CGEvent::new_keyboard_event(source, KEY_C, false),
    ) else {
        tracing::error!("could not create keyboard events");
        return false;
    };
    down.set_flags(CGEventFlags::CGEventFlagCommand);
    up.set_flags(CGEventFlags::CGEventFlagCommand);
    down.post(CGEventTapLocation::HID);
    std::thread::sleep(Duration::from_millis(8));
    up.post(CGEventTapLocation::HID);
    true
}

pub struct MacCapturer;

impl Capturer for MacCapturer {
    fn capture(&self, budget: Duration, clipboard_fallback: bool) -> Captured {
        let app = frontmost_app();

        // Without Accessibility we cannot synthesise anything; say so rather than
        // presenting whatever happens to be on the clipboard as if it were a selection.
        if !accessibility_trusted() {
            let text = read_text();
            return Captured {
                source: if text.is_empty() || !clipboard_fallback {
                    CaptureSource::Empty
                } else {
                    CaptureSource::ClipboardFallback
                },
                text: if clipboard_fallback { text } else { String::new() },
                app,
            };
        }

        let before = snapshot();
        if !send_copy() {
            return Captured { text: read_text(), source: CaptureSource::ClipboardFallback, app };
        }

        let deadline = Instant::now() + budget;
        let mut changed = false;
        while Instant::now() < deadline {
            if change_count() != before.change_count {
                changed = true;
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }

        if !changed {
            // No selection existed, so ⌘C was a no-op and the clipboard is untouched —
            // nothing to restore.
            tracing::debug!("no pasteboard change within budget; treating as no selection");
            let text = read_text();
            return Captured {
                source: if text.is_empty() || !clipboard_fallback {
                    CaptureSource::Empty
                } else {
                    CaptureSource::ClipboardFallback
                },
                text: if clipboard_fallback { text } else { String::new() },
                app,
            };
        }

        let text = read_text();
        let after = change_count();

        // Restoring is deferred so the HUD can be shown without waiting on it.
        std::thread::spawn(move || {
            std::thread::sleep(SETTLE);
            restore(&before, after);
        });

        Captured { text, source: CaptureSource::Selection, app }
    }

    fn can_synthesize(&self) -> bool {
        accessibility_trusted()
    }
}

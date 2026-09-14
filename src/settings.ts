import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./styles/settings.css";

type EngineOptions = {
  normalizePersian: boolean;
  softUnwrap: boolean;
  stripBackticks: boolean;
  tabWidth: number;
};

type Settings = {
  shortcut: string;
  engine: EngineOptions;
  dismissOnBlur: boolean;
  clipboardFallback: boolean;
  fontSize: number;
};

const el = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const shortcutButton = el<HTMLButtonElement>("shortcut");
const status = el("status");
const axStatus = el("ax-status");

let settings: Settings;
let recording = false;

function say(message: string, error = false): void {
  status.textContent = message;
  status.classList.toggle("error", error);
  if (!error) setTimeout(() => (status.textContent = ""), 2200);
}

async function persist(next: Settings): Promise<void> {
  try {
    settings = await invoke<Settings>("save_settings", { settings: next });
    say("Saved");
  } catch (error) {
    // A rejected shortcut leaves the old one registered, so reflect that rather than
    // showing a binding that is not actually active.
    say(String(error), true);
    render();
  }
}

/** Translate a keyboard event into the shortcut syntax the global-shortcut plugin parses. */
function toShortcut(event: KeyboardEvent): string | null {
  const parts: string[] = [];
  if (event.ctrlKey) parts.push("Control");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  if (event.metaKey) parts.push("Command");

  const code = event.code;
  let key: string | null = null;
  if (/^Key[A-Z]$/.test(code)) key = code.slice(3);
  else if (/^Digit[0-9]$/.test(code)) key = code.slice(5);
  else if (/^F\d{1,2}$/.test(code)) key = code;
  else if (code === "Space") key = "Space";

  // A modifier-only combination cannot be a trigger.
  if (!key || parts.length === 0) return null;
  parts.push(key);
  return parts.join("+");
}

shortcutButton.addEventListener("click", () => {
  recording = true;
  shortcutButton.classList.add("recording");
  shortcutButton.textContent = "Press keys…";
});

window.addEventListener("keydown", (event) => {
  if (!recording) return;
  event.preventDefault();
  if (event.key === "Escape") {
    recording = false;
    shortcutButton.classList.remove("recording");
    render();
    return;
  }
  const shortcut = toShortcut(event);
  if (!shortcut) return;
  recording = false;
  shortcutButton.classList.remove("recording");
  void persist({ ...settings, shortcut });
});

function bindCheck(id: keyof Settings | keyof EngineOptions, engine: boolean): void {
  const input = el<HTMLInputElement>(id as string);
  input.addEventListener("change", () => {
    const next = engine
      ? { ...settings, engine: { ...settings.engine, [id]: input.checked } }
      : { ...settings, [id]: input.checked };
    void persist(next as Settings);
  });
}

bindCheck("softUnwrap", true);
bindCheck("normalizePersian", true);
bindCheck("stripBackticks", true);
bindCheck("dismissOnBlur", false);
bindCheck("clipboardFallback", false);

el<HTMLInputElement>("fontSize").addEventListener("change", (event) => {
  const value = Number((event.target as HTMLInputElement).value);
  if (Number.isFinite(value)) void persist({ ...settings, fontSize: value });
});

el("ax-open").addEventListener("click", () => void invoke("open_accessibility_settings"));

function render(): void {
  shortcutButton.textContent = settings.shortcut;
  el<HTMLInputElement>("softUnwrap").checked = settings.engine.softUnwrap;
  el<HTMLInputElement>("normalizePersian").checked = settings.engine.normalizePersian;
  el<HTMLInputElement>("stripBackticks").checked = settings.engine.stripBackticks;
  el<HTMLInputElement>("dismissOnBlur").checked = settings.dismissOnBlur;
  el<HTMLInputElement>("clipboardFallback").checked = settings.clipboardFallback;
  el<HTMLInputElement>("fontSize").value = String(settings.fontSize);
}

/// Permission can be granted while this window is open, so re-check whenever the user
/// comes back to it rather than showing a stale "required" forever.
async function refreshAccessibility(): Promise<void> {
  const trusted = await invoke<boolean>("accessibility_status");
  axStatus.textContent = trusted
    ? "Accessibility access granted"
    : "Accessibility access is required to capture a selection";
  axStatus.classList.toggle("warn", !trusted);
}

async function init(): Promise<void> {
  settings = await invoke<Settings>("get_settings");
  render();
  await refreshAccessibility();

  // Reflect changes made by another settings window immediately.
  void listen<Settings>("rtlens://settings", (event) => {
    settings = event.payload;
    if (!recording) render();
  });
  window.addEventListener("focus", () => void refreshAccessibility());
}

void init();

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { renderDoc, type Doc } from "./render";
import "./styles/hud.css";

type Settings = {
  fontSize: number;
};

type DocPayload = {
  doc: Doc;
  settings: Settings;
  sourceApp: string | null;
  canCapture: boolean;
};

const el = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

const shell = el("shell");
const docEl = el("doc");
const empty = el("empty");
const emptyTitle = el("empty-title");
const emptyHint = el("empty-hint");
const emptyAction = el<HTMLButtonElement>("empty-action");
const close = el<HTMLButtonElement>("close");

emptyAction.addEventListener("click", () => {
  void invoke("open_accessibility_settings");
});

/**
 * #doc's 12px top and bottom padding, plus #shell's 0.5px border on each edge,
 * rounded up so sizing never lands the content one pixel into a scrollbar.
 */
const DOC_PADDING = 26;

/** Longest we will wait on webfonts before sizing the window anyway. */
const FONT_DEADLINE = 80;

function measureContentHeight(): number {
  if (!empty.hidden) {
    return Math.max(empty.offsetHeight, 100);
  }
  let content = 0;
  for (const child of Array.from(docEl.children)) {
    content += (child as HTMLElement).offsetHeight;
  }
  return content;
}

/**
 * Measure the rendered content and ask Rust to size and reveal the window.
 *
 * Waiting for `fonts.ready` matters: Persian and monospace faces have very different
 * metrics from the fallbacks, so measuring first would size the window to the wrong text.
 */
async function present(): Promise<void> {
  // `document.fonts.ready` can never settle here: the window is still hidden, and WKWebView
  // defers font loading for a window it is not painting. Awaiting it unguarded deadlocks
  // the measurement, so it is raced against a deadline and re-measured once it does settle.
  await Promise.race([
    document.fonts.ready.catch(() => undefined),
    new Promise((resolve) => setTimeout(resolve, FONT_DEADLINE)),
  ]);
  const height = measureContentHeight() + DOC_PADDING;
  void invoke("hud_ready", { height });
  remeasureWhenFontsLoad(height);
}

/**
 * Persian and monospace faces have very different metrics from the fallbacks, so once they
 * really are loaded the content may be a different height than what we sized the window to.
 */
function remeasureWhenFontsLoad(previous: number): void {
  void document.fonts.ready
    .then(() => {
      const height = measureContentHeight() + DOC_PADDING;
      if (Math.abs(height - previous) > 4) void invoke("hud_ready", { height });
    })
    .catch(() => undefined);
}

function showDocument(payload: DocPayload): void {
  shell.style.setProperty("--rtl-size", `${payload.settings.fontSize}px`);

  const hasContent = payload.doc.lines.some((line) =>
    line.segments.some((segment) => segment.text.trim() !== "")
  );

  if (!payload.canCapture) {
    docEl.replaceChildren();
    empty.hidden = false;
    emptyAction.hidden = false;
    emptyTitle.textContent = "Accessibility access required";
    emptyHint.textContent =
      "RTLens needs Accessibility access to copy your selected text from terminals and apps (via ⌘C). No keystrokes are recorded or stored.";
    emptyAction.textContent = "Grant Permission…";
    void invoke("request_accessibility_permission");
  } else if (!hasContent) {
    docEl.replaceChildren();
    empty.hidden = false;
    emptyAction.hidden = true;
    emptyTitle.textContent = "No selection detected";
    emptyHint.textContent = "Select some text, then press the RTLens shortcut.";
  } else {
    empty.hidden = true;
    emptyAction.hidden = true;
    renderDoc(docEl, payload.doc);
  }

  void present();
}

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    event.preventDefault();
    void invoke("hide_hud");
  }
});

close.addEventListener("click", () => void invoke("hide_hud"));

function applySettings(next: Settings): void {
  shell.style.setProperty("--rtl-size", `${next.fontSize}px`);
}

void listen<DocPayload>("rtlens://doc", (event) => showDocument(event.payload));
void listen<Settings>("rtlens://settings", (event) => applySettings(event.payload));

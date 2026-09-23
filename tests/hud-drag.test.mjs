import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const index = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const renderer = readFileSync(new URL("../src/render.ts", import.meta.url), "utf8");
const layout = readFileSync(new URL("../src/styles/layout.css", import.meta.url), "utf8");

test("the frameless HUD keeps a drag surface without swallowing text selection", () => {
  assert.match(index, /id="shell"[^>]*data-tauri-drag-region="deep"/);
  assert.match(index, /id="close"[^>]*data-tauri-drag-region="false"/s);
  assert.match(index, /id="drag-handle"[^>]*data-tauri-drag-region="true"/s);
  assert.match(renderer, /function preserveTextSelection\(/);
  assert.match(renderer, /preserveTextSelection\(span\)/);
  assert.match(renderer, /preserveTextSelection\(cellEl\)/);
  assert.match(renderer, /preserveTextSelection\(content\)/);
});

test("the HUD close control is wired to the hide command", () => {
  const main = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");
  assert.match(main, /const close = el<HTMLButtonElement>\("close"\)/);
  assert.match(main, /close\.addEventListener\("click", \(\) => void invoke\("hide_hud"\)\)/);
});

test("the HUD pin control is wired to set_dismiss_on_blur and excluded from dragging", () => {
  assert.match(index, /id="pin-btn"[^>]*data-tauri-drag-region="false"/s);
  const main = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");
  assert.match(main, /const pinBtn = el<HTMLButtonElement>\("pin-btn"\)/);
  assert.match(main, /invoke\("set_dismiss_on_blur", \{ dismissOnBlur: !nextPinned \}\)/);
});

test("RTL popup text is anchored to the right edge of the available width", () => {
  assert.match(renderer, /if \(!line\.boxed && line\.dir === "rtl"\)/);
  assert.match(layout, /\.rtl-doc \{[^}]*width: 100%;/s);
  assert.match(layout, /\.rtl-content\[dir="rtl"\] \{[^}]*text-align: right;/s);
  assert.match(layout, /\.rtl-content\[dir="ltr"\] \{[^}]*text-align: left;/s);
  assert.match(layout, /\.rtl-table \{[^}]*width: max-content;/s);
});

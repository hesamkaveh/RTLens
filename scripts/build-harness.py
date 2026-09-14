#!/usr/bin/env python3
"""Build a before/after visual harness for the RTLens text engine.

"Before" is the raw capture rendered the way a terminal effectively renders it: one bidi
paragraph per line, first-strong direction, neutral characters left to fend for themselves.
"After" is the same capture through RTLens. Anything the engine fixes shows up as a
difference between the two columns.
"""
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "crates/rtlens-core/tests/fixtures"
DUMP = ROOT / "target/debug/rtlens-dump"
OUT = ROOT / "target/harness.html"

LAYOUT_CSS = (ROOT / "src/styles/layout.css").read_text(encoding="utf-8")

PAGE_CSS = """
:root { color-scheme: dark;
  /* Mirrors layout.css so both columns are compared on identical typography. */
  --rtl-font-mono: "JetBrains Mono", ui-monospace, "SF Mono", Menlo, Consolas,
                   "Vazirmatn", "SF Arabic", Tahoma, monospace; }
body { margin:0; padding:28px; background:#0f1115; color:#e8e8ea;
       font-family: ui-sans-serif, system-ui, sans-serif; }
h1 { font-size:19px; font-weight:650; margin:0 0 4px; letter-spacing:-0.01em; }
.sub { color:#8b93a1; font-size:13px; margin:0 0 26px; }
.case { margin-bottom:30px; border:1px solid #242833; border-radius:12px; overflow:hidden;
        background:#151821; }
.case > h2 { font-size:12px; font-weight:650; margin:0; padding:9px 14px; color:#aeb6c4;
             background:#1b1f2a; border-bottom:1px solid #242833;
             font-family: ui-monospace, Menlo, monospace; letter-spacing:0.02em; }
.cols { display:grid; grid-template-columns:1fr 1fr; }
.col { padding:14px 16px; min-width:0; overflow-x:auto; }
.col + .col { border-left:1px solid #242833; }
.tag { font-size:10px; font-weight:700; letter-spacing:0.09em; text-transform:uppercase;
       margin-bottom:10px; font-family: ui-monospace, Menlo, monospace; }
.before .tag { color:#e5766b; }
.after  .tag { color:#5fb87a; }
pre.raw { margin:0; font-family: var(--rtl-font-mono); font-size:14px; line-height:1.9;
          white-space:pre-wrap; overflow-wrap:anywhere;
          /* What a terminal does: per-line first-strong, neutrals unprotected. */
          unicode-bidi:plaintext; }
"""


def fragment(path: pathlib.Path) -> str:
    return subprocess.run(
        [str(DUMP), "--fragment"],
        stdin=path.open("rb"),
        capture_output=True,
        check=True,
    ).stdout.decode("utf-8")


def esc(s: str) -> str:
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def raw_preview(path: pathlib.Path) -> str:
    """Strip escape codes only, so 'before' shows layout breakage rather than ANSI noise."""
    text = subprocess.run(
        [str(DUMP), "--text", "--no-unwrap", "--no-normalize"],
        stdin=path.open("rb"),
        capture_output=True,
        check=True,
    ).stdout.decode("utf-8")
    return esc(text)


def main() -> int:
    if not DUMP.exists():
        print(f"missing {DUMP}; run: cargo build -p rtlens-core --bin rtlens-dump", file=sys.stderr)
        return 1

    # With no arguments every fixture is rendered. Name one or more (with or without
    # the .txt) to narrow it down while chasing a single case.
    wanted = {a.removesuffix(".txt") for a in sys.argv[1:]}
    paths = [p for p in sorted(FIXTURES.glob("*.txt")) if not wanted or p.stem in wanted]
    if not paths:
        print(f"no fixture matched {sorted(wanted)}", file=sys.stderr)
        return 1

    cases = []
    for path in paths:
        cases.append(
            f'<div class="case"><h2>{esc(path.name)}</h2><div class="cols">'
            f'<div class="col before"><div class="tag">before &middot; terminal behaviour</div>'
            f'<pre class="raw">{raw_preview(path)}</pre></div>'
            f'<div class="col after"><div class="tag">after &middot; rtlens</div>'
            f"{fragment(path)}</div></div></div>"
        )

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(
        "<!doctype html><html><head><meta charset='utf-8'>"
        "<title>RTLens engine harness</title>"
        f"<style>{PAGE_CSS}\n{LAYOUT_CSS}</style></head><body>"
        "<h1>RTLens engine harness</h1>"
        "<p class='sub'>Left column renders each line as a terminal would. "
        "Right column is the same capture through the RTLens engine.</p>"
        + "\n".join(cases)
        + "</body></html>",
        encoding="utf-8",
    )
    print(OUT)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

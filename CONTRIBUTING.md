# Contributing to RTLens

Thanks for helping. The short version: the engine is where contributions are
cheapest and most valuable, and everything it needs is headless.

## Setup

- Rust (stable), via [rustup](https://rustup.rs). If `cargo` is not on your PATH,
  rustup was installed with `--no-modify-path`; add it with
  `. "$HOME/.cargo/env"`.
- Node 20.19+ or 22.12+ with pnpm.

```sh
pnpm install
```

## Before opening a PR

`pnpm check` (TypeScript + workspace-wide clippy, warnings as errors), `pnpm test`
(engine tests), and `pnpm test:hud` must pass. CI runs all three, plus `cargo fmt
--all -- --check` — formatting is governed by the committed `rustfmt.toml`, so
plain `cargo fmt --all` produces the expected result.

## Working on the engine

`crates/rtlens-core` has no GUI or OS dependency. Iterate on it directly:

```sh
cargo test -p rtlens-core        # unit + snapshot tests
cargo insta review               # accept intentional snapshot changes
```

Regression reports are easiest to turn into fixtures: save the raw capture as a
`.txt` under `crates/rtlens-core/tests/fixtures`, then register it with one line
in `crates/rtlens-core/tests/engine.rs`:

```rust
snapshot_fixture!(columnar_table, "columnar_table.txt");
```

The first run writes a `.snap.new` next to the committed snapshots; `cargo insta
review` turns it into the `.snap` that gets committed. A fixture with no
`snapshot_fixture!` line is never run by anything, so don't leave one behind.

To eyeball a fix before/after:

```sh
pnpm harness                     # writes target/harness.html, open it in a browser

# narrow it to the cases you are working on
python3 scripts/build-harness.py claude_box_persian
```

## Working on the shell

`src-tauri` is only the shell (hotkey, capture, window, tray) and effectively
requires macOS to develop against — the Windows/Linux capture paths are written
but unverified, so don't be surprised if they need work; that work is welcome,
just say which platform you tested on.

## Reporting a broken capture

Open an issue with the *raw* text (before RTLens touches it) — copied straight
from the terminal — plus what it should look like. Screenshots help because the
failure mode is visual.

## What this changes

<!-- One or two sentences. Link the issue if there is one. -->

## Checks

- [ ] `pnpm check` passes (tsc + workspace clippy, warnings as errors)
- [ ] `pnpm test` passes
- [ ] `cargo fmt --all` run (formatting comes from the committed `rustfmt.toml`)

## For an engine change

- [ ] A fixture under `crates/rtlens-core/tests/fixtures` covers it
- [ ] Snapshot changes were reviewed with `cargo insta review`, and the new
      output is correct rather than merely different

## For a shell change

Which platform did you test on? The Windows and Linux capture paths ship
unverified, so say what you actually ran.

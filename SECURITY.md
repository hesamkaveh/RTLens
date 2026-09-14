# Security Policy

## Reporting a vulnerability

Please do **not** open a public issue for a security problem.

Report it through [GitHub private vulnerability
reporting](https://github.com/hesamkaveh/RTLens/security/advisories/new), which
reaches the maintainers without making the report public. You should get an
initial response within a week.

If you are reporting something that involves a captured document, redact it
first — see [What RTLens can see](#what-rtlens-can-see) below for why the raw
text is worth treating as sensitive.

## Supported versions

RTLens is pre-1.0. Only the latest release gets fixes; there are no maintained
branches for older versions.

## What RTLens can see

The permissions RTLens asks for are broad by necessity, so it is worth being
explicit about what they do and do not allow.

**Accessibility access (macOS).** Required to synthesise ⌘C into the frontmost
application. RTLens uses the OS hotkey registry
(`RegisterEventHotKey`), not an event tap, so it never observes the keystrokes
you type — a registered hotkey can only fire for its own combination. It does
not read window contents, and it does not use `AXSelectedText`.

**The pasteboard.** A capture snapshots the pasteboard, synthesises a copy,
reads the result, and restores the snapshot. Two consequences follow:

- Whatever you had copied is briefly replaced, and clipboard managers
  (Raycast, Maccy, Paste) will record both the transient copy and the restore.
  This is inherent to the technique — there is no way to copy without the
  clipboard noticing.
- If the pasteboard changes again while RTLens holds it, the restore is
  skipped rather than clobbering another application's write.

**Captured text.** Selections are held in memory for the life of the HUD and are
never written to disk. Only the settings in
`~/Library/Application Support/com.hesamkaveh.rtlens/settings.json` persist, and no
captured text is among them.

## What RTLens does not do

- **No network access of any kind.** RTLens makes no requests, has no telemetry,
  no crash reporting and no update check. The webview is locked to a
  `default-src 'self'` content security policy, so a rendered document cannot
  cause a fetch even if it contains markup.
- **No code execution from captured text.** Input is treated as text throughout;
  the renderer builds DOM nodes from string content and never interprets a
  capture as HTML.

## Build provenance

Releases are built by the `release.yml` GitHub Actions workflow from a tagged
commit — never uploaded by hand. The workflow requires a signing certificate and
verifies the resulting macOS bundle, but the release is **not notarised with
Apple**. Gatekeeper may therefore block the first launch, and you should expect to
open it once from the context menu or clear the quarantine flag. Verify a download
against the workflow run attached to the release if you want to confirm where it
came from.

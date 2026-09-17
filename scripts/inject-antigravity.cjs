#!/usr/bin/env node
// RTLens live RTL injector for Antigravity, over the Chrome DevTools Protocol.
//
// Antigravity must be started with remote debugging enabled (it then writes
// DevToolsActivePort). While that port is open, any local process can drive
// the app — only enable it on a machine you trust.
//
//   node scripts/inject-antigravity.cjs          inject into open windows once
//   node scripts/inject-antigravity.cjs --watch  stay attached; survives reloads
//                                                and picks up new windows
//   node scripts/inject-antigravity.cjs --install    run --watch at login (launchd)
//   node scripts/inject-antigravity.cjs --uninstall  remove the login agent
'use strict';

const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');

const PORT_FILE = process.env.RTLENS_DEVTOOLS_PORT_FILE || path.join(
    process.env.HOME || '',
    'Library/Application Support/Antigravity/DevToolsActivePort'
);
const CDP_TIMEOUT_MS = 5000;
const POLL_MS = 2000;

const CLIENT_FILE = path.join(__dirname, '../integrations/antigravity/client.cjs');

function getClientScript() {
    return fs.readFileSync(CLIENT_FILE, 'utf8');
}

// ---- CDP plumbing ----------------------------------------------------------

async function listPages() {
    if (!fs.existsSync(PORT_FILE)) {
        throw new Error('Antigravity is not running with remote debugging (DevToolsActivePort not found)');
    }
    const port = fs.readFileSync(PORT_FILE, 'utf8').split('\n')[0].trim();
    let res;
    try {
        res = await fetch(`http://127.0.0.1:${port}/json`, { signal: AbortSignal.timeout(CDP_TIMEOUT_MS) });
    } catch {
        throw new Error('Antigravity is not reachable on its debug port (quit or still starting)');
    }
    const targets = await res.json();
    return targets.filter((t) => t.type === 'page' && t.webSocketDebuggerUrl && !t.url.startsWith('devtools://'));
}

function connect(page) {
    return new Promise((resolve, reject) => {
        const ws = new WebSocket(page.webSocketDebuggerUrl);
        const pending = new Map();
        let nextId = 1;
        const timer = setTimeout(() => {
            ws.close();
            reject(new Error(`timed out connecting to "${page.title}"`));
        }, CDP_TIMEOUT_MS);

        const session = {
            ws,
            send(method, params = {}) {
                return new Promise((res, rej) => {
                    const id = nextId++;
                    const t = setTimeout(() => {
                        pending.delete(id);
                        rej(new Error(`${method} timed out`));
                    }, CDP_TIMEOUT_MS);
                    pending.set(id, { res, rej, t });
                    ws.send(JSON.stringify({ id, method, params }));
                });
            },
            close() {
                ws.close();
            },
        };

        ws.onopen = () => {
            clearTimeout(timer);
            resolve(session);
        };
        ws.onmessage = (event) => {
            const msg = JSON.parse(event.data);
            const p = msg.id && pending.get(msg.id);
            if (!p) return;
            pending.delete(msg.id);
            clearTimeout(p.t);
            if (msg.error) p.rej(new Error(msg.error.message));
            else if (msg.result.exceptionDetails) p.rej(new Error(msg.result.exceptionDetails.text));
            else p.res(msg.result);
        };
        const fail = () => {
            clearTimeout(timer);
            for (const p of pending.values()) {
                clearTimeout(p.t);
                p.rej(new Error('connection closed'));
            }
            pending.clear();
            reject(new Error(`could not connect to "${page.title}"`));
        };
        ws.onerror = fail;
        ws.onclose = fail;
    });
}

// Runs the script in the current document and, when persistent, registers it
// for every future document of this target so reloads keep RTL support.
async function attach(page, { persistent }) {
    const session = await connect(page);
    try {
        const script = getClientScript();
        if (persistent) {
            // Without Page.enable the registration is accepted but never runs on reload.
            await session.send('Page.enable');
            await session.send('Page.addScriptToEvaluateOnNewDocument', { source: script });
        }
        await session.send('Runtime.evaluate', { expression: script });
    } catch (err) {
        session.close();
        throw err;
    }
    return session;
}

async function inject() {
    const pages = await listPages();
    if (pages.length === 0) throw new Error('No Antigravity window found');
    for (const page of pages) {
        const session = await attach(page, { persistent: false });
        session.close();
        console.log(`[RTLens] Injected into "${page.title}"`);
    }
}

async function watch() {
    console.log('[RTLens] Watching Antigravity windows (Ctrl+C to stop)...');
    const sessions = new Map(); // target id -> session
    let lastError = null;

    for (;;) {
        try {
            const pages = await listPages();
            for (const page of pages) {
                if (sessions.has(page.id)) continue;
                const session = await attach(page, { persistent: true });
                sessions.set(page.id, session);
                session.ws.addEventListener('close', () => sessions.delete(page.id));
                console.log(`${new Date().toISOString()} [RTLens] Attached to "${page.title}"`);
            }
            lastError = null;
        } catch (err) {
            if (err.message !== lastError) console.error(`${new Date().toISOString()} [RTLens] ${err.message}`);
            lastError = err.message;
        }
        await new Promise((r) => setTimeout(r, POLL_MS));
    }
}

// Node 20 ships WebSocket behind a flag; re-exec with it instead of adding a dependency.
function ensureWebSocket() {
    if (typeof globalThis.WebSocket === 'function') return;
    if (process.execArgv.includes('--experimental-websocket')) {
        console.error('[RTLens] This Node has no WebSocket support; use Node >= 20.10.');
        process.exit(1);
    }
    const r = spawnSync(
        process.execPath,
        ['--experimental-websocket', ...process.execArgv, __filename, ...process.argv.slice(2)],
        { stdio: 'inherit' }
    );
    process.exit(r.status === null ? 1 : r.status);
}

// ---- Login agent ------------------------------------------------------------

const AGENT_LABEL = 'com.rtlens.antigravity';
const AGENT_PLIST = path.join(process.env.HOME || '', 'Library/LaunchAgents', `${AGENT_LABEL}.plist`);
const AGENT_LOG = path.join(process.env.HOME || '', 'Library/Logs/RTLens/antigravity.log');

function launchctl(...args) {
    return spawnSync('launchctl', args, { encoding: 'utf8' });
}

function uninstall({ quiet = false } = {}) {
    launchctl('bootout', `gui/${process.getuid()}/${AGENT_LABEL}`);
    const existed = fs.existsSync(AGENT_PLIST);
    if (existed) fs.rmSync(AGENT_PLIST);
    if (!quiet) console.log(existed ? `[RTLens] Removed ${AGENT_PLIST}` : '[RTLens] Login agent was not installed');
}

// The plist pins today's node binary and this checkout's path: reinstall after
// moving the repo or switching Node versions.
function install() {
    const xml = (v) => v.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    const plist = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>${AGENT_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>${xml(process.execPath)}</string>
        <string>${xml(__filename)}</string>
        <string>--watch</string>
    </array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>ThrottleInterval</key><integer>30</integer>
    <key>ProcessType</key><string>Background</string>
    <key>LowPriorityIO</key><true/>
    <key>StandardOutPath</key><string>${xml(AGENT_LOG)}</string>
    <key>StandardErrorPath</key><string>${xml(AGENT_LOG)}</string>
</dict>
</plist>
`;
    uninstall({ quiet: true });
    fs.mkdirSync(path.dirname(AGENT_PLIST), { recursive: true });
    fs.mkdirSync(path.dirname(AGENT_LOG), { recursive: true });
    fs.writeFileSync(AGENT_PLIST, plist);
    const r = launchctl('bootstrap', `gui/${process.getuid()}`, AGENT_PLIST);
    if (r.status !== 0) {
        console.error(`[RTLens] launchctl bootstrap failed: ${(r.stderr || '').trim()}`);
        process.exit(1);
    }
    console.log(`[RTLens] Installed ${AGENT_LABEL}; it runs at login. Log: ${AGENT_LOG}`);
}

if (require.main === module) {
    const has = (...flags) => flags.some((f) => process.argv.includes(f));
    if (has('--install')) {
        install();
    } else if (has('--uninstall')) {
        uninstall();
    } else {
        ensureWebSocket();
        if (has('--watch', '-w')) {
            watch();
        } else {
            inject()
                .then(() => console.log('[RTLens] Done. Reloading the window removes it; use --watch to persist.'))
                .catch((err) => {
                    console.error(`[RTLens] ${err.message}`);
                    process.exit(1);
                });
        }
    }
}

module.exports = { inject, watch, getClientScript };

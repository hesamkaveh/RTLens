//! Live RTL support for Antigravity's agent chat.
//!
//! Antigravity is an Electron app. When it runs with remote debugging enabled it writes
//! `DevToolsActivePort`, and every window is reachable over the Chrome DevTools Protocol.
//! RTLens attaches to each window, registers the page script for every future document
//! (so reloads and restarts keep it) and runs it in the current one.
//!
//! A registered script only runs while its DevTools connection stays open, which is why
//! each window holds a connection for as long as the feature is on.

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tungstenite::{Message, WebSocket};

/// Shared with the dev CLI in `scripts/inject-antigravity.cjs`.
const CLIENT_SCRIPT: &str = include_str!("../../integrations/antigravity/client.cjs");
const TEARDOWN_SCRIPT: &str = "window.__rtlens && window.__rtlens.teardown()";

/// How often to look for new Antigravity windows. One loopback HTTP request per tick.
const POLL: Duration = Duration::from_secs(2);
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// How long an attached connection blocks before re-checking whether it was switched off.
const SESSION_TICK: Duration = Duration::from_millis(500);

struct Shared {
    enabled: AtomicBool,
    port_file: Option<PathBuf>,
    /// Target ids with a session thread, whether or not it has attached yet.
    claimed: Mutex<HashSet<String>>,
    /// Windows the script is actually running in.
    attached: AtomicUsize,
    last_error: Mutex<Option<String>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    enabled: bool,
    attached: usize,
    error: Option<String>,
}

pub struct Antigravity {
    shared: Arc<Shared>,
}

impl Antigravity {
    pub fn start(enabled: bool) -> Self {
        Self::start_with(enabled, default_port_file())
    }

    fn start_with(enabled: bool, port_file: Option<PathBuf>) -> Self {
        let shared = Arc::new(Shared {
            enabled: AtomicBool::new(enabled),
            port_file,
            claimed: Mutex::default(),
            attached: AtomicUsize::new(0),
            last_error: Mutex::default(),
        });
        let poller = shared.clone();
        if let Err(e) = std::thread::Builder::new().name("antigravity".into()).spawn(move || poll(poller)) {
            tracing::error!("could not start the Antigravity watcher: {e}");
        }
        Self { shared }
    }

    /// Turning it off removes the script from every attached window, not just future ones.
    pub fn set_enabled(&self, enabled: bool) {
        self.shared.enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn status(&self) -> Status {
        Status {
            enabled: self.shared.enabled.load(Ordering::SeqCst),
            attached: self.shared.attached.load(Ordering::SeqCst),
            error: self.shared.last_error.lock().expect("error lock").clone(),
        }
    }
}

fn poll(shared: Arc<Shared>) {
    loop {
        let error = if shared.enabled.load(Ordering::SeqCst) {
            match list_pages(shared.port_file.as_deref()) {
                Ok(pages) => {
                    for page in pages {
                        if !shared.claimed.lock().expect("claimed lock").insert(page.id.clone()) {
                            continue;
                        }
                        let session_shared = shared.clone();
                        let spawned = std::thread::Builder::new()
                            .name("antigravity-window".into())
                            .spawn(move || session(session_shared, page));
                        if let Err(e) = spawned {
                            tracing::error!("could not attach to an Antigravity window: {e}");
                        }
                    }
                    None
                }
                Err(e) => Some(e),
            }
        } else {
            None
        };
        *shared.last_error.lock().expect("error lock") = error;
        std::thread::sleep(POLL);
    }
}

fn default_port_file() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| {
            PathBuf::from(home).join("Library/Application Support/Antigravity/DevToolsActivePort")
        })
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|config| config.join("Antigravity/DevToolsActivePort"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(|appdata| PathBuf::from(appdata).join("Antigravity/DevToolsActivePort"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

struct Page {
    id: String,
    title: String,
    ws_url: String,
}

fn list_pages(port_file: Option<&std::path::Path>) -> Result<Vec<Page>, String> {
    const NOT_RUNNING: &str = "Antigravity is not running with remote debugging enabled";
    let path = port_file.ok_or(NOT_RUNNING)?;
    let contents = std::fs::read_to_string(path).map_err(|_| NOT_RUNNING)?;
    let port: u16 = contents
        .lines()
        .next()
        .and_then(|line| line.trim().parse().ok())
        .ok_or("Antigravity's DevToolsActivePort file is malformed")?;

    let body = http_get(port, "/json/list")
        .map_err(|_| "Antigravity is not reachable on its debug port (quit or still starting)".to_string())?;
    let targets: Vec<Value> =
        serde_json::from_slice(&body).map_err(|e| format!("unexpected answer from Antigravity: {e}"))?;

    Ok(targets
        .iter()
        .filter(|t| t["type"] == "page" && !t["url"].as_str().unwrap_or_default().starts_with("devtools://"))
        .filter_map(|t| {
            Some(Page {
                id: t["id"].as_str()?.to_owned(),
                title: t["title"].as_str().unwrap_or_default().to_owned(),
                ws_url: t["webSocketDebuggerUrl"].as_str()?.to_owned(),
            })
        })
        .collect())
}

/// A minimal GET against the loopback DevTools endpoint, which always answers with
/// `Content-Length`. Not worth an HTTP client dependency.
fn http_get(port: u16, path: &str) -> std::io::Result<Vec<u8>> {
    let mut stream = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), IO_TIMEOUT)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")?;

    let invalid = |msg: &str| std::io::Error::new(ErrorKind::InvalidData, msg.to_owned());
    let mut raw = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let n = stream.read(&mut chunk)?;
        raw.extend_from_slice(&chunk[..n]);
        if let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
            if !head.starts_with("http/1.1 200") && !head.starts_with("http/1.0 200") {
                return Err(invalid("non-200 response"));
            }
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok());
            let body = &raw[split + 4..];
            match length {
                Some(length) if body.len() >= length => return Ok(body[..length].to_vec()),
                None if n == 0 => return Ok(body.to_vec()),
                _ => {}
            }
        }
        if n == 0 {
            return Err(invalid("connection closed early"));
        }
    }
}

struct Cdp {
    ws: WebSocket<TcpStream>,
    next_id: u64,
}

impl Cdp {
    fn connect(ws_url: &str) -> Result<Self, String> {
        let authority = ws_url
            .strip_prefix("ws://")
            .and_then(|rest| rest.split('/').next())
            .ok_or_else(|| format!("unexpected debugger url {ws_url}"))?;
        let stream = TcpStream::connect(authority).map_err(|e| e.to_string())?;
        stream.set_read_timeout(Some(IO_TIMEOUT)).map_err(|e| e.to_string())?;
        stream.set_write_timeout(Some(IO_TIMEOUT)).map_err(|e| e.to_string())?;
        let (ws, _) = tungstenite::client(ws_url, stream).map_err(|e| e.to_string())?;
        Ok(Self { ws, next_id: 0 })
    }

    /// Send a command and wait for its reply, dropping any events that arrive meanwhile.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let request = json!({ "id": id, "method": method, "params": params }).to_string();
        self.ws.send(Message::text(request)).map_err(|e| format!("{method}: {e}"))?;
        loop {
            let message = self.ws.read().map_err(|e| format!("{method}: {e}"))?;
            let Message::Text(text) = message else { continue };
            let Ok(reply) = serde_json::from_str::<Value>(text.as_str()) else { continue };
            if reply["id"] != id {
                continue;
            }
            if let Some(error) = reply.get("error") {
                return Err(format!("{method}: {}", error["message"]));
            }
            if let Some(exception) = reply["result"].get("exceptionDetails") {
                return Err(format!("{method}: {}", exception["text"]));
            }
            return Ok(reply["result"].clone());
        }
    }
}

fn session(shared: Arc<Shared>, page: Page) {
    if let Err(e) = attach_and_hold(&shared, &page) {
        tracing::debug!("Antigravity window \"{}\" detached: {e}", page.title);
    }
    shared.claimed.lock().expect("claimed lock").remove(&page.id);
}

/// Counts a window as attached for exactly as long as this guard lives.
struct AttachedGuard<'a>(&'a AtomicUsize);

impl<'a> AttachedGuard<'a> {
    fn new(count: &'a AtomicUsize) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(count)
    }
}

impl Drop for AttachedGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn attach_and_hold(shared: &Shared, page: &Page) -> Result<(), String> {
    let mut cdp = Cdp::connect(&page.ws_url)?;
    // Without Page.enable the registration is accepted but never runs on reload.
    cdp.call("Page.enable", json!({}))?;
    let registered = cdp.call("Page.addScriptToEvaluateOnNewDocument", json!({ "source": CLIENT_SCRIPT }))?;
    cdp.call("Runtime.evaluate", json!({ "expression": CLIENT_SCRIPT }))?;
    let _attached = AttachedGuard::new(&shared.attached);
    tracing::info!("RTL support attached to Antigravity window \"{}\"", page.title);

    // Hold the connection, draining events, until the window goes away or the feature is
    // switched off. A read timeout here is just the tick, not a failure.
    cdp.ws.get_mut().set_read_timeout(Some(SESSION_TICK)).map_err(|e| e.to_string())?;
    while shared.enabled.load(Ordering::SeqCst) {
        match cdp.ws.read() {
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) => return Err(e.to_string()),
        }
    }

    cdp.ws.get_mut().set_read_timeout(Some(IO_TIMEOUT)).map_err(|e| e.to_string())?;
    if let Some(identifier) = registered["identifier"].as_str() {
        cdp.call("Page.removeScriptToEvaluateOnNewDocument", json!({ "identifier": identifier }))?;
    }
    cdp.call("Runtime.evaluate", json!({ "expression": TEARDOWN_SCRIPT }))?;
    let _ = cdp.ws.close(None);
    let _ = cdp.ws.flush();
    tracing::info!("RTL support removed from Antigravity window \"{}\"", page.title);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::{Child, Command};
    use std::time::Instant;

    /// Headless Chrome stands in for Antigravity: same DevTools protocol and port file.
    struct Browser {
        child: Child,
        profile: PathBuf,
    }

    impl Browser {
        fn launch(chrome: &str, profile: &Path, url: &str) -> Self {
            let _ = std::fs::remove_file(profile.join("DevToolsActivePort"));
            let child = Command::new(chrome)
                .args(["--headless=new", "--remote-debugging-port=0", "--no-first-run"])
                .arg(format!("--user-data-dir={}", profile.display()))
                .arg(url)
                .spawn()
                .expect("launch chrome");
            let browser = Self { child, profile: profile.to_owned() };
            wait_for("browser", || list_pages(Some(&browser.port_file())).is_ok_and(|p| !p.is_empty()));
            browser
        }

        fn port_file(&self) -> PathBuf {
            self.profile.join("DevToolsActivePort")
        }

        /// Talk to the first page over a separate DevTools connection.
        fn call(&self, method: &str, params: Value) -> Value {
            let page = list_pages(Some(&self.port_file())).expect("pages").remove(0);
            let mut cdp = Cdp::connect(&page.ws_url).expect("connect");
            let result = cdp.call(method, params).expect(method);
            let _ = cdp.ws.close(None);
            result
        }

        fn eval(&self, expression: &str) -> Value {
            self.call("Runtime.evaluate", json!({ "expression": expression, "returnByValue": true }))
                ["result"]["value"]
                .clone()
        }

        fn reload(&self) {
            self.call("Page.reload", json!({}));
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    impl Drop for Browser {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn wait_for(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    const INSTALLED: &str =
        "typeof window.__rtlens === 'object' && !!document.getElementById('rtlens-smart-rtl-style')";
    const MARKED: &str = "String(document.querySelector('p').getAttribute('data-rtlens-dir'))";

    /// Needs Chrome: `RTLENS_TEST_CHROME=/path/to/chrome cargo test -p rtlens -- --ignored`.
    #[test]
    #[ignore]
    fn attaches_survives_reload_and_restart_then_detaches() {
        let chrome = std::env::var("RTLENS_TEST_CHROME").expect("RTLENS_TEST_CHROME");
        let dir = std::env::temp_dir().join(format!("rtlens-antigravity-{}", std::process::id()));
        let profile = dir.join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let page = dir.join("page.html");
        std::fs::write(&page, "<meta charset=utf-8><p>RTLens یک ابزار برای نمایش متن است</p>").unwrap();
        let url = format!("file://{}", page.display());

        let mut browser = Browser::launch(&chrome, &profile, &url);
        let watcher = Antigravity::start_with(true, Some(browser.port_file()));
        wait_for("attach", || watcher.status().attached == 1);
        wait_for("script", || browser.eval(INSTALLED) == true);
        assert_eq!(browser.eval(MARKED), "rtl");

        browser.reload();
        wait_for("script after reload", || browser.eval(INSTALLED) == true);

        drop(browser);
        wait_for("detach on quit", || watcher.status().attached == 0);
        browser = Browser::launch(&chrome, &profile, &url);
        wait_for("script after restart", || browser.eval(INSTALLED) == true);
        browser.reload();
        wait_for("script after restart and reload", || browser.eval(INSTALLED) == true);
        assert_eq!(browser.eval(MARKED), "rtl");

        watcher.set_enabled(false);
        wait_for("detach when disabled", || watcher.status().attached == 0);
        assert_eq!(browser.eval(INSTALLED), false);
        assert_eq!(browser.eval(MARKED), "null");
        browser.reload();
        assert_eq!(browser.eval(INSTALLED), false, "script must not return after disabling");

        drop(browser);
        let _ = std::fs::remove_dir_all(dir);
    }
}

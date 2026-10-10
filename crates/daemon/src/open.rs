//! Opens http and https URLs from sandboxes in the browser on the host.
//!
//! Each running sandbox gets one relay: a shell loop, started as an exec stream, that reads URLs
//! from the FIFO at [`FIFO_PATH`] and echoes them to stdout. The `firebrick-open` stand-in for
//! `xdg-open` in the `firebrick-base` image writes to that FIFO. fbkd reads the relay's output
//! line by line, checks each URL and hands it to an [`Opener`]. It never opens a URL whose host
//! is the host itself or its local network, or one the sandbox's enforced egress rules don't
//! allow, so the browser can't be used to get around them.

use crate::network;
use firebrick_spec::NetworkSpec;
use microsandbox::Sandbox;
use microsandbox::sandbox::exec::{ExecEvent, ExecHandle};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use url::{Host, Url};

/// Guest path of the FIFO that the stand-in writes URLs to.
pub const FIFO_PATH: &str = "/tmp/.firebrick/open.fifo";

/// The longest URL fbkd opens, in bytes.
const MAX_URL_LEN: usize = 8 * 1024;

/// The most URLs fbkd opens for one sandbox within [`OPEN_WINDOW`].
const MAX_OPENS: usize = 5;

/// The window [`MAX_OPENS`] applies to.
const OPEN_WINDOW: Duration = Duration::from_secs(10);

/// The most bytes of the relay's stderr fbkd keeps to log when the relay exits.
const MAX_STDERR: usize = 1024;

/// The relay script. It creates the FIFO's directory and the FIFO when they're missing, then
/// echoes each line written to the FIFO. It keeps the FIFO open for writing itself, so reads
/// don't end when a writer closes it.
const RELAY_SCRIPT: &str = r#"d=/tmp/.firebrick
f="$d/open.fifo"
mkdir -p -m 700 "$d" || exit 1
[ -p "$f" ] || mkfifo -m 600 "$f" || exit 1
exec 3<>"$f"
while IFS= read -r line <&3; do printf '%s\n' "$line"; done"#;

/// The name the relay script runs under (`$0`), which shows up in the guest's process list.
const RELAY_NAME: &str = "firebrick-open-relay";

/// Opens a URL on the host.
pub trait Opener: Send + Sync {
    /// Opens the URL without waiting for the program that shows it.
    fn open(&self, url: &str) -> io::Result<()>;
}

/// Opens URLs in the user's default browser with `xdg-open` on Linux and `open` on macOS.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostOpener;

impl Opener for HostOpener {
    fn open(&self, url: &str) -> io::Result<()> {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };

        // Tokio reaps the dropped child in the background, so it doesn't stay behind as a zombie.
        tokio::process::Command::new(program)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(drop)
    }
}

/// Why fbkd doesn't open a URL from a sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rejection {
    /// It isn't an http or https URL of at most [`MAX_URL_LEN`] bytes without whitespace or
    /// control characters.
    Invalid,
    /// Its host is the host itself or its local network.
    LocalHost,
    /// The sandbox's egress rules don't allow its host.
    NotAllowed,
}

/// Checks a line from the relay and returns the URL to open, as the browser would read it.
fn check_url(line: &[u8], network: &NetworkSpec) -> Result<Url, Rejection> {
    let text = std::str::from_utf8(line)
        .ok()
        .filter(|text| is_valid_url(text))
        .ok_or(Rejection::Invalid)?;
    let url = Url::parse(text).map_err(|_| Rejection::Invalid)?;
    let host = url.host().ok_or(Rejection::Invalid)?;

    if is_local(&host) {
        return Err(Rejection::LocalHost);
    }

    if !network::allows_host(network, &host) {
        return Err(Rejection::NotAllowed);
    }

    Ok(url)
}

/// Whether the text starts with `http://` or `https://` in any case, has no whitespace or
/// control characters and is at most [`MAX_URL_LEN`] bytes long.
fn is_valid_url(text: &str) -> bool {
    let has_scheme = ["http://", "https://"].iter().any(|scheme| {
        text.get(..scheme.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
    });

    has_scheme
        && text.len() <= MAX_URL_LEN
        && !text.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Whether the host is `localhost`, or an IP address of the host itself or its local network.
fn is_local(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            name == "localhost" || name.ends_with(".localhost")
        }
        Host::Ipv4(ip) => is_local_ipv4(*ip),
        Host::Ipv6(ip) => is_local_ipv6(*ip),
    }
}

/// Whether the IPv4 address is unspecified, loopback, private, shared (CGNAT), link-local or
/// broadcast.
fn is_local_ipv4(ip: Ipv4Addr) -> bool {
    let [first, second, ..] = ip.octets();

    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || (first == 100 && second & 0xc0 == 64)
}

/// Whether the IPv6 address is unspecified, loopback, unique local or link-local, or an
/// IPv4-mapped local address.
fn is_local_ipv6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];

    ip.is_unspecified()
        || ip.is_loopback()
        || first & 0xfe00 == 0xfc00
        || first & 0xffc0 == 0xfe80
        || ip.to_ipv4_mapped().is_some_and(is_local_ipv4)
}

/// Splits the relay's output into lines. A line longer than [`MAX_URL_LEN`] is cut off just past
/// the limit, so it fails validation without being buffered whole.
#[derive(Debug, Default)]
struct LineSplitter {
    line: Vec<u8>,
}

impl LineSplitter {
    /// Adds output and returns the lines it completes, without their `\n`.
    fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        let mut parts = data.split(|&byte| byte == b'\n');
        // `split` always yields at least one part, which continues the current line.
        self.extend(parts.next().unwrap_or_default());

        parts
            .map(|part| {
                let line = std::mem::take(&mut self.line);
                self.extend(part);
                line
            })
            .collect()
    }

    /// Adds a part without `\n` to the current line, up to one byte past [`MAX_URL_LEN`].
    fn extend(&mut self, part: &[u8]) {
        let room = (MAX_URL_LEN + 1).saturating_sub(self.line.len());
        self.line.extend_from_slice(&part[..part.len().min(room)]);
    }
}

/// Allows at most [`MAX_OPENS`] URLs within [`OPEN_WINDOW`].
#[derive(Debug, Default)]
struct RateLimit {
    opened: VecDeque<Instant>,
}

impl RateLimit {
    /// Records an open at `now` and returns `true` when the limit allows it.
    fn allow(&mut self, now: Instant) -> bool {
        while self
            .opened
            .front()
            .is_some_and(|opened| now.duration_since(*opened) >= OPEN_WINDOW)
        {
            self.opened.pop_front();
        }

        if self.opened.len() >= MAX_OPENS {
            return false;
        }

        self.opened.push_back(now);
        true
    }
}

/// Handles the output of the relay of one sandbox.
struct RelayOutput {
    sandbox: String,
    network: NetworkSpec,
    opener: Arc<dyn Opener>,
    lines: LineSplitter,
    limit: RateLimit,
    stderr: Vec<u8>,
}

impl RelayOutput {
    /// Creates the handler for the relay of the sandbox with the egress rules.
    fn new(sandbox: &str, network: NetworkSpec, opener: Arc<dyn Opener>) -> Self {
        Self {
            sandbox: sandbox.to_string(),
            network,
            opener,
            lines: LineSplitter::default(),
            limit: RateLimit::default(),
            stderr: Vec::new(),
        }
    }

    /// Opens the URLs in the relay's output, and logs when the relay fails or exits.
    fn handle_event(&mut self, event: ExecEvent) {
        match event {
            ExecEvent::Stdout(data) => self.handle_stdout(&data),
            ExecEvent::Stderr(data) => self.keep_stderr(&data),
            ExecEvent::Failed(failed) => {
                let sandbox = &self.sandbox;
                tracing::warn!(error = ?failed, "failed to start the URL relay of sandbox {sandbox}");
            }
            ExecEvent::Exited { code } => self.log_exit(code),
            _ => {}
        }
    }

    /// Opens the URLs on the lines the output completes.
    fn handle_stdout(&mut self, data: &[u8]) {
        for line in self.lines.push(data) {
            self.handle_line(&line, Instant::now());
        }
    }

    /// Opens the URL on a line when it passes the checks and the rate limit, and logs a warning
    /// without the URL when it doesn't or when opening fails.
    fn handle_line(&mut self, line: &[u8], now: Instant) {
        let sandbox = &self.sandbox;
        let url = match check_url(line, &self.network) {
            Ok(url) => url,
            Err(rejection) => return log_rejection(sandbox, rejection),
        };

        if !self.limit.allow(now) {
            tracing::warn!("ignoring a URL from sandbox {sandbox}: it opens URLs too often");
            return;
        }

        match self.opener.open(url.as_str()) {
            Ok(()) => tracing::info!("opened a URL from sandbox {sandbox} on the host"),
            Err(err) => tracing::warn!("failed to open a URL from sandbox {sandbox}: {err}"),
        }
    }

    /// Keeps the first [`MAX_STDERR`] bytes of the relay's stderr.
    fn keep_stderr(&mut self, data: &[u8]) {
        let room = MAX_STDERR.saturating_sub(self.stderr.len());
        self.stderr.extend_from_slice(&data[..data.len().min(room)]);
    }

    /// Logs the relay's exit, as a warning with its stderr when it failed.
    fn log_exit(&self, code: i32) {
        let sandbox = &self.sandbox;

        if code == 0 {
            tracing::info!("URL relay of sandbox {sandbox} exited");
        } else {
            let stderr = String::from_utf8_lossy(&self.stderr);
            tracing::warn!("URL relay of sandbox {sandbox} exited with {code}: {stderr}");
        }
    }
}

/// Logs why a URL from the sandbox isn't opened, without the URL.
fn log_rejection(sandbox: &str, rejection: Rejection) {
    match rejection {
        Rejection::Invalid => tracing::warn!("ignoring invalid URL from sandbox {sandbox}"),
        Rejection::LocalHost => tracing::warn!(
            "ignoring a URL from sandbox {sandbox}: it points to the host or its local network"
        ),
        Rejection::NotAllowed => tracing::warn!(
            "ignoring a URL from sandbox {sandbox}: its network rules don't allow the host"
        ),
    }
}

/// The relays of the running sandboxes, at most one per sandbox.
pub struct Relays {
    opener: Arc<dyn Opener>,
    // The generation of the relay that runs for each sandbox, so a relay that ends only removes
    // its own entry, not that of a newer relay.
    running: Arc<Mutex<HashMap<String, u64>>>,
    next_generation: AtomicU64,
}

impl Relays {
    /// Creates a registry whose relays open URLs with `opener`.
    pub fn new(opener: Arc<dyn Opener>) -> Self {
        Self {
            opener,
            running: Arc::default(),
            next_generation: AtomicU64::new(0),
        }
    }

    /// Starts the relay of the connected, running sandbox unless it already has one. The relay
    /// checks URLs against the egress rules the sandbox has now. Logs a warning when the relay
    /// can't start.
    pub async fn ensure(&self, sb: &Sandbox) {
        let name = sb.name();
        let Some(generation) = self.claim(name) else {
            return;
        };

        let started = sb
            .exec_stream_with("sh", |e| e.args(["-c", RELAY_SCRIPT, RELAY_NAME]))
            .await;

        match started {
            Ok(handle) => {
                let entry = RelayEntry {
                    running: self.running.clone(),
                    name: name.to_string(),
                    generation,
                };
                let network = network::rules_of(&sb.config().spec);
                let output = RelayOutput::new(name, network, self.opener.clone());
                tokio::spawn(run_relay(handle, entry, output));
            }
            Err(err) => {
                tracing::warn!("failed to start the URL relay of sandbox {name}: {err}");
                release(&self.running, name, generation);
            }
        }
    }

    /// Forgets the relay of the sandbox, so the next [`Relays::ensure`] starts a new one. Used
    /// when the sandbox stops, because its relay's exec stream may end only after that.
    pub fn forget(&self, name: &str) {
        lock(&self.running).remove(name);
    }

    /// Registers a relay for the sandbox and returns its generation, or `None` when the sandbox
    /// already has one.
    fn claim(&self, name: &str) -> Option<u64> {
        let mut running = lock(&self.running);

        if running.contains_key(name) {
            return None;
        }

        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        running.insert(name.to_string(), generation);

        Some(generation)
    }
}

/// The registry entry of a running relay.
struct RelayEntry {
    running: Arc<Mutex<HashMap<String, u64>>>,
    name: String,
    generation: u64,
}

/// Handles the relay's output until its exec stream ends, then removes its entry.
async fn run_relay(mut handle: ExecHandle, entry: RelayEntry, mut output: RelayOutput) {
    while let Some(event) = handle.recv().await {
        output.handle_event(event);
    }

    release(&entry.running, &entry.name, entry.generation);
}

/// Removes the entry of the sandbox when it still belongs to the relay with the generation.
fn release(running: &Mutex<HashMap<String, u64>>, name: &str, generation: u64) {
    let mut running = lock(running);

    if running.get(name) == Some(&generation) {
        running.remove(name);
    }
}

/// Locks the registry. Its map stays consistent when a holder panics, so a poisoned lock is
/// used as is.
fn lock(running: &Mutex<HashMap<String, u64>>) -> std::sync::MutexGuard<'_, HashMap<String, u64>> {
    running.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records the URLs it's asked to open, and fails when `fail` is set.
    #[derive(Default)]
    struct RecordingOpener {
        urls: Mutex<Vec<String>>,
        fail: bool,
    }

    impl Opener for RecordingOpener {
        fn open(&self, url: &str) -> io::Result<()> {
            self.urls.lock().unwrap().push(url.to_string());

            self.fail
                .then(|| io::Error::new(io::ErrorKind::NotFound, "no browser"))
                .map_or(Ok(()), Err)
        }
    }

    impl RecordingOpener {
        fn urls(&self) -> Vec<String> {
            self.urls.lock().unwrap().clone()
        }
    }

    #[test]
    fn accepts_http_and_https_in_any_case() {
        assert!(is_valid_url("http://example.com"));
        assert!(is_valid_url("https://github.com/login/device"));
        assert!(is_valid_url("HTTPS://EXAMPLE.COM/path?q=1#x"));
        assert!(is_valid_url("hTtP://localhost:8080/callback"));
    }

    #[test]
    fn rejects_other_schemes_and_missing_scheme() {
        assert!(!is_valid_url("file:///etc/passwd"));
        assert!(!is_valid_url("javascript:alert(1)"));
        assert!(!is_valid_url("ftp://example.com"));
        assert!(!is_valid_url("example.com"));
        assert!(!is_valid_url("http:/example.com"));
        assert!(!is_valid_url(" https://example.com"));
        assert!(!is_valid_url(""));
    }

    #[test]
    fn rejects_whitespace_and_control_characters() {
        assert!(!is_valid_url("https://example.com/a b"));
        assert!(!is_valid_url("https://example.com/\t"));
        assert!(!is_valid_url("https://example.com/\r"));
        assert!(!is_valid_url("https://example.com/\u{1b}[31m"));
        assert!(!is_valid_url("https://example.com/\u{7f}"));
        assert!(!is_valid_url("https://example.com/\u{85}"));
    }

    #[test]
    fn rejects_urls_over_the_length_limit() {
        let prefix = "https://example.com/";
        let at_limit = format!("{prefix}{}", "a".repeat(MAX_URL_LEN - prefix.len()));
        let over_limit = format!("{at_limit}a");

        assert!(is_valid_url(&at_limit));
        assert!(!is_valid_url(&over_limit));
    }

    /// Returns a handler for the relay of sandbox `sb` with the rules, and its opener.
    fn relay_output(network: NetworkSpec) -> (RelayOutput, Arc<RecordingOpener>) {
        let opener = Arc::new(RecordingOpener::default());
        (RelayOutput::new("sb", network, opener.clone()), opener)
    }

    fn enforced(allow: &[&str]) -> NetworkSpec {
        NetworkSpec {
            enforce: true,
            allow: allow.iter().map(|rule| rule.parse().unwrap()).collect(),
            ..NetworkSpec::default()
        }
    }

    fn check(line: &str, network: &NetworkSpec) -> Result<String, Rejection> {
        check_url(line.as_bytes(), network).map(String::from)
    }

    #[test]
    fn check_url_returns_the_url_as_the_browser_reads_it() {
        let network = NetworkSpec::default();

        assert_eq!(
            check("HTTPS://GitHub.com/login/device", &network),
            Ok("https://github.com/login/device".into())
        );
        assert_eq!(
            check("http://example.com:8080/a?b=c#d", &network),
            Ok("http://example.com:8080/a?b=c#d".into())
        );
    }

    #[test]
    fn check_url_rejects_invalid_urls() {
        let network = NetworkSpec::default();

        for line in [
            "file:///etc/passwd",
            "https://",
            "https://exa mple.com",
            "http://[::1",
        ] {
            assert_eq!(check(line, &network), Err(Rejection::Invalid), "{line}");
        }
        assert_eq!(
            check_url(b"https://example.com/\xff", &network),
            Err(Rejection::Invalid)
        );
    }

    #[test]
    fn check_url_rejects_the_host_and_its_local_network() {
        let network = NetworkSpec::default();
        let local = [
            "http://localhost:8080/callback",
            "http://LOCALHOST./",
            "http://app.localhost/",
            "http://127.0.0.1/",
            "http://127.1/",
            "http://2130706433/",
            "http://0x7f.0.0.1/",
            "http://0.0.0.0/",
            "http://10.1.2.3/",
            "http://172.16.0.1/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://100.64.0.1/",
            "http://[::1]/",
            "http://[::]/",
            "http://[fd00::1]/",
            "http://[fe80::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://user@127.0.0.1/",
        ];

        for line in local {
            assert_eq!(check(line, &network), Err(Rejection::LocalHost), "{line}");
        }
        assert!(check("http://8.8.8.8/", &network).is_ok());
        assert!(check("http://[2001:db8::1]/", &network).is_ok());
        assert!(check("http://localhost.example/", &network).is_ok());
    }

    #[test]
    fn check_url_applies_enforced_network_rules() {
        let network = enforced(&["github.com", "*.anthropic.com"]);

        assert!(check("https://github.com/login/device", &network).is_ok());
        assert!(check("https://console.anthropic.com/oauth", &network).is_ok());
        assert_eq!(
            check("https://attacker.example/?d=secret", &network),
            Err(Rejection::NotAllowed)
        );
        assert_eq!(
            check("https://github.com.attacker.example/", &network),
            Err(Rejection::NotAllowed)
        );
        assert_eq!(
            check("https://github.com@attacker.example/", &network),
            Err(Rejection::NotAllowed)
        );
    }

    #[test]
    fn rate_limit_allows_a_burst_then_waits_for_the_window() {
        let mut limit = RateLimit::default();
        let start = Instant::now();

        for _ in 0..MAX_OPENS {
            assert!(limit.allow(start));
        }
        assert!(!limit.allow(start + OPEN_WINDOW / 2));
        assert!(limit.allow(start + OPEN_WINDOW));
    }

    #[test]
    fn handle_line_opens_valid_url() {
        let (mut output, opener) = relay_output(NetworkSpec::default());

        output.handle_line(b"https://example.com", Instant::now());

        assert_eq!(opener.urls(), ["https://example.com/"]);
    }

    #[test]
    fn handle_line_ignores_rejected_lines() {
        let (mut output, opener) = relay_output(enforced(&["example.com"]));
        let now = Instant::now();

        output.handle_line(b"file:///etc/passwd", now);
        output.handle_line(b"https://example.com/\xff", now);
        output.handle_line(b"", now);
        output.handle_line(b"http://localhost:3000/", now);
        output.handle_line(b"https://example.org/", now);

        assert!(opener.urls().is_empty());
    }

    #[test]
    fn handle_line_stops_opening_urls_over_the_rate_limit() {
        let (mut output, opener) = relay_output(NetworkSpec::default());
        let now = Instant::now();

        for _ in 0..=MAX_OPENS {
            output.handle_line(b"https://example.com/", now);
        }

        assert_eq!(opener.urls().len(), MAX_OPENS);
    }

    #[test]
    fn handle_line_survives_failing_opener() {
        let opener = Arc::new(RecordingOpener {
            fail: true,
            ..Default::default()
        });
        let mut output = RelayOutput::new("sb", NetworkSpec::default(), opener.clone());

        output.handle_line(b"https://example.com/", Instant::now());
        output.handle_line(b"https://example.org/", Instant::now());

        assert_eq!(
            opener.urls(),
            ["https://example.com/", "https://example.org/"]
        );
    }

    #[test]
    fn handle_event_opens_urls_split_over_chunks() {
        let (mut output, opener) = relay_output(NetworkSpec::default());

        output.handle_event(ExecEvent::Stdout("https://exa".into()));
        output.handle_event(ExecEvent::Stdout(
            "mple.com/\nhttps://example.org/\n".into(),
        ));

        assert_eq!(
            opener.urls(),
            ["https://example.com/", "https://example.org/"]
        );
    }

    #[test]
    fn handle_event_keeps_the_start_of_stderr() {
        let (mut output, _) = relay_output(NetworkSpec::default());

        output.handle_event(ExecEvent::Stderr("mkfifo: not found\n".into()));
        output.handle_event(ExecEvent::Stderr(vec![b'x'; 2 * MAX_STDERR].into()));

        assert!(output.stderr.starts_with(b"mkfifo: not found\n"));
        assert_eq!(output.stderr.len(), MAX_STDERR);
    }

    #[test]
    fn line_splitter_joins_chunks_and_splits_lines() {
        let mut lines = LineSplitter::default();

        assert!(lines.push(b"https://exa").is_empty());
        assert_eq!(
            lines.push(b"mple.com\nhttps://b.example\nhttps://c"),
            [
                b"https://example.com".to_vec(),
                b"https://b.example".to_vec()
            ]
        );
        assert_eq!(lines.push(b".example\n"), [b"https://c.example".to_vec()]);
    }

    #[test]
    fn line_splitter_cuts_long_lines_so_they_fail_validation() {
        let mut lines = LineSplitter::default();
        let long = format!("https://example.com/{}\n", "a".repeat(2 * MAX_URL_LEN));

        let split = lines.push(long.as_bytes());

        assert_eq!(split.len(), 1);
        assert_eq!(split[0].len(), MAX_URL_LEN + 1);
        assert_eq!(
            check_url(&split[0], &NetworkSpec::default()),
            Err(Rejection::Invalid)
        );
    }

    #[test]
    fn forget_lets_a_new_relay_claim_the_sandbox() {
        let relays = Relays::new(Arc::new(HostOpener));

        let first = relays.claim("sb").unwrap();
        assert_eq!(relays.claim("sb"), None);

        relays.forget("sb");
        let second = relays.claim("sb").unwrap();

        // The first relay ending doesn't remove the entry of the second.
        release(&relays.running, "sb", first);
        assert_eq!(relays.claim("sb"), None);
        release(&relays.running, "sb", second);
        assert!(relays.claim("sb").is_some());
    }
}

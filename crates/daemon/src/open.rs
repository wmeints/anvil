//! Opens http and https URLs from sandboxes in the browser on the host.
//!
//! Each running sandbox gets one relay: a shell loop, started as an exec stream, that reads URLs
//! from the FIFO at [`FIFO_PATH`] and echoes them to stdout. The `firebrick-open` stand-in for
//! `xdg-open` in the `firebrick-base` image writes to that FIFO. fbkd reads the relay's output
//! line by line, validates each URL and hands it to an [`Opener`].

use microsandbox::Sandbox;
use microsandbox::sandbox::exec::{ExecEvent, ExecHandle};
use std::collections::HashMap;
use std::io;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// Guest path of the FIFO that the stand-in writes URLs to.
pub const FIFO_PATH: &str = "/tmp/.firebrick/open.fifo";

/// The longest URL fbkd opens, in bytes.
pub const MAX_URL_LEN: usize = 8 * 1024;

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
pub const RELAY_NAME: &str = "firebrick-open-relay";

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
        let mut child = Command::new(program)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        // Reap the opener in the background, so it doesn't stay behind as a zombie.
        std::thread::spawn(move || child.wait());

        Ok(())
    }
}

/// Whether fbkd may open the URL: it starts with `http://` or `https://` in any case, has no
/// whitespace or control characters and is at most [`MAX_URL_LEN`] bytes long.
pub fn is_valid_url(url: &str) -> bool {
    let has_scheme = ["http://", "https://"].iter().any(|scheme| {
        url.get(..scheme.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
    });

    has_scheme
        && url.len() <= MAX_URL_LEN
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Opens the URL on a line from the relay of a sandbox when it's valid, and logs a warning when
/// it isn't or when opening fails.
pub fn handle_line(sandbox: &str, line: &[u8], opener: &dyn Opener) {
    let Some(url) = std::str::from_utf8(line)
        .ok()
        .filter(|url| is_valid_url(url))
    else {
        tracing::warn!("ignoring invalid URL from sandbox {sandbox}");
        return;
    };

    match opener.open(url) {
        Ok(()) => tracing::info!("opened a URL from sandbox {sandbox} on the host"),
        Err(err) => tracing::warn!("failed to open a URL from sandbox {sandbox}: {err}"),
    }
}

/// Splits the relay's output into lines. A line longer than [`MAX_URL_LEN`] is cut off just past
/// the limit, so it fails validation without being buffered whole.
#[derive(Debug, Default)]
pub struct LineSplitter {
    line: Vec<u8>,
}

impl LineSplitter {
    /// Adds output and returns the lines it completes, without their `\n`.
    pub fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
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

    /// Starts the relay of the running sandbox with the name, unless it already has one. Logs a
    /// warning when the relay can't start.
    pub async fn ensure(&self, name: &str) {
        let Some(generation) = self.claim(name) else {
            return;
        };

        match start_relay(name).await {
            Ok(handle) => {
                let entry = RelayEntry {
                    running: self.running.clone(),
                    name: name.to_string(),
                    generation,
                };
                tokio::spawn(run_relay(handle, entry, self.opener.clone()));
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

/// Connects to the sandbox and starts the relay script in it.
async fn start_relay(name: &str) -> Result<ExecHandle, microsandbox::MicrosandboxError> {
    let sb = Sandbox::get(name).await?.connect().await?;

    sb.exec_stream_with("sh", |e| e.args(["-c", RELAY_SCRIPT, RELAY_NAME]))
        .await
}

/// Opens the URLs the relay echoes until its exec stream ends, then removes its entry.
async fn run_relay(mut handle: ExecHandle, entry: RelayEntry, opener: Arc<dyn Opener>) {
    let mut lines = LineSplitter::default();

    while let Some(event) = handle.recv().await {
        handle_event(event, &mut lines, &entry.name, opener.as_ref());
    }

    release(&entry.running, &entry.name, entry.generation);
}

/// Opens the URLs in the relay's output and logs when the relay fails or exits.
fn handle_event(event: ExecEvent, lines: &mut LineSplitter, sandbox: &str, opener: &dyn Opener) {
    match event {
        ExecEvent::Stdout(data) => lines
            .push(&data)
            .iter()
            .for_each(|line| handle_line(sandbox, line, opener)),
        ExecEvent::Failed(failed) => {
            tracing::warn!(error = ?failed, "failed to start the URL relay of sandbox {sandbox}");
        }
        ExecEvent::Exited { code } => {
            tracing::info!("URL relay of sandbox {sandbox} exited with {code}");
        }
        _ => {}
    }
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

    #[test]
    fn handle_line_opens_valid_url() {
        let opener = RecordingOpener::default();

        handle_line("sb", b"https://example.com", &opener);

        assert_eq!(opener.urls(), ["https://example.com"]);
    }

    #[test]
    fn handle_line_ignores_invalid_lines() {
        let opener = RecordingOpener::default();

        handle_line("sb", b"file:///etc/passwd", &opener);
        handle_line("sb", b"https://example.com/\xff", &opener);
        handle_line("sb", b"", &opener);

        assert!(opener.urls().is_empty());
    }

    #[test]
    fn handle_line_survives_failing_opener() {
        let opener = RecordingOpener {
            fail: true,
            ..Default::default()
        };

        handle_line("sb", b"https://example.com", &opener);
        handle_line("sb", b"https://example.org", &opener);

        assert_eq!(
            opener.urls(),
            ["https://example.com", "https://example.org"]
        );
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
        let opener = RecordingOpener::default();
        handle_line("sb", &split[0], &opener);
        assert!(opener.urls().is_empty());
    }

    #[test]
    fn forget_lets_a_new_relay_claim_the_sandbox() {
        let relays = Relays::new(Arc::new(RecordingOpener::default()));

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

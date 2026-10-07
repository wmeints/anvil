use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use crate::api::sandbox_management_service_client::SandboxManagementServiceClient;
use anyhow::{Context, Result, bail};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tokio::time::{Instant, sleep};
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

const DAEMON_BINARY: &str = "anvild";
const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(5);
const DAEMON_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Connects to the daemon over its unix socket, starting the daemon when needed.
pub async fn connect() -> Result<SandboxManagementServiceClient<Channel>> {
    let path = anvil_utils::socket_path();

    // Note: the HTTP endpoint isn't actually used. It will only show up in the authority header.
    let channel = Endpoint::try_from("http://localhost")
        .unwrap()
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = path.clone();
            async move {
                let stream = ensure_daemon(&path).await.map_err(std::io::Error::other)?;
                Ok::<_, std::io::Error>(TokioIo::new(stream))
            }
        }))
        .await?;

    Ok(SandboxManagementServiceClient::new(channel))
}

/// Connects to the daemon socket, spawning the daemon and waiting for it when it isn't running.
pub async fn ensure_daemon(socket_path: &Path) -> Result<UnixStream> {
    if let Ok(stream) = UnixStream::connect(socket_path).await {
        return Ok(stream);
    }

    // The socket file may be left behind by a daemon that didn't shut down cleanly.
    // The daemon refuses to start when the socket exists, so clean it up first.
    if socket_path.exists() {
        std::fs::remove_file(socket_path)
            .with_context(|| format!("failed to remove stale socket {}", socket_path.display()))?;
    }

    let child = Command::new(daemon_binary())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .context("failed to start the anvil daemon")?;

    wait_for_daemon(socket_path, child).await
}

/// Waits until the spawned daemon listens on the socket, failing when it exits or times out.
async fn wait_for_daemon(socket_path: &Path, mut child: Child) -> Result<UnixStream> {
    let deadline = Instant::now() + DAEMON_START_TIMEOUT;

    loop {
        if let Ok(stream) = UnixStream::connect(socket_path).await {
            return Ok(stream);
        }

        if let Some(status) = child.try_wait()? {
            bail!("the anvil daemon exited unexpectedly ({status})");
        }

        if Instant::now() >= deadline {
            bail!(
                "timed out waiting for the anvil daemon to listen on {}",
                socket_path.display()
            );
        }

        sleep(DAEMON_POLL_INTERVAL).await;
    }
}

/// Returns the daemon binary next to the current executable, or falls back to looking it up on `PATH`.
fn daemon_binary() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(DAEMON_BINARY)))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(DAEMON_BINARY))
}

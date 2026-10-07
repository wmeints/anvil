use anyhow::{Context, Result, bail};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

use anvil_daemon::secrets::{self, SecretStore};
use anvil_daemon::{runtime, server, ssh};

/// Sets up logging and runs the daemon on its unix socket until shutdown.
#[tokio::main]
async fn main() -> Result<()> {
    let log_dir = anvil_utils::log_dir();
    std::fs::create_dir_all(&log_dir)?;
    let (log_writer, _log_guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(&log_dir, "anvild.log"));

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(fmt::layer().with_ansi(false).with_writer(log_writer))
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    // Check the socket before touching the runtime, so a second daemon can't replace the
    // runtime under the one that is already serving.
    let socket_path = anvil_utils::socket_path();
    if socket_path.exists() {
        bail!(server::ServerError::SocketAlreadyInUse());
    }

    let msb_config =
        microsandbox::config::config().context("failed to load the microsandbox configuration")?;
    runtime::ensure(&msb_config).await?;
    secrets::protect_database(&msb_config.home())
        .context("failed to protect the microsandbox database")?;
    ssh::ensure_keys()?;
    server::sync_ssh_config().await;

    tracing::info!(path = %log_dir.display(), "writing logs");
    tracing::info!(
        path = socket_path.to_str().unwrap(),
        "listening on unix socket"
    );
    Ok(server::run(&socket_path, SecretStore::new(anvil_utils::secrets_path())).await?)
}

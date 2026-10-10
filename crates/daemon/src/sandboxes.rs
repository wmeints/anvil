//! Sandbox management on top of microsandbox: the sandbox lifecycle, the workspace's mise
//! tools, the secrets of sandboxes, SSH host names, the generated SSH config, the editors'
//! Remote-SSH settings and Zed's remote projects.

use crate::mise::{self, MiseError};
use crate::network;
use crate::secrets::{self, Secret, SecretStore};
use crate::ssh;
use crate::vscode;
use crate::zed;
use firebrick_spec::NetworkSpec;
use microsandbox::sandbox::{HostPermissions, SandboxBuilder, SandboxHandle, SandboxStatus};
use microsandbox::{MicrosandboxError, Sandbox};
use std::collections::HashSet;
use std::fmt;
use std::path::Path;
use std::time::Duration;
use thiserror::Error;

/// How long a sandbox gets to shut down gracefully before it is killed.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// Guest path of the disk that holds Docker's data.
const DOCKER_DATA_PATH: &str = "/var/lib/docker";

/// Errors of sandbox management. Each variant carries the message the client sees.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum SandboxError {
    /// The sandbox or secret doesn't exist.
    #[error("{0}")]
    NotFound(String),
    /// The request has an invalid value.
    #[error("{0}")]
    InvalidArgument(String),
    /// The sandbox isn't in a state that allows the operation.
    #[error("{0}")]
    FailedPrecondition(String),
    /// microsandbox or the secret store failed.
    #[error("{0}")]
    Internal(String),
}

impl SandboxError {
    fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    fn invalid_argument(message: impl Into<String>) -> Self {
        Self::InvalidArgument(message.into())
    }

    fn failed_precondition(message: impl Into<String>) -> Self {
        Self::FailedPrecondition(message.into())
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}

/// The resources a sandbox gets when it is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resources {
    /// Number of vCPUs.
    pub cpus: u8,
    /// Memory in MiB.
    pub memory_mib: u32,
    /// Size of the Docker volume in MiB.
    pub docker_volume_mib: u32,
}

/// The sandbox to start, and the workspace and image to create it from when it doesn't exist.
#[derive(Debug, Clone, Copy)]
pub struct StartSandbox<'a> {
    /// Name of the sandbox.
    pub name: &'a str,
    /// Absolute host path of the workspace to mount.
    pub workspace: &'a str,
    /// Image to create the sandbox from, or empty for the default image.
    pub image: &'a str,
    /// Whether the image's `/sbin/init` runs as PID 1 when the sandbox is created.
    pub init: bool,
    /// Whether the sandbox trusts and installs its workspace's mise tools on start. Stored when
    /// the sandbox is created.
    pub mise: bool,
    /// Egress rules of the sandbox, applied when it is created.
    pub network: &'a NetworkSpec,
}

/// The name, status, SSH host name and workspace path of a sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxInfo {
    /// Name of the sandbox.
    pub name: String,
    /// Status of the sandbox.
    pub status: SandboxStatus,
    /// SSH host name of the sandbox, if it has one.
    pub hostname: Option<String>,
    /// Guest path the workspace is mounted at, e.g. `/workspaces/project`, if it has one.
    pub workspace_path: Option<String>,
}

impl SandboxInfo {
    fn of(handle: &SandboxHandle) -> Self {
        Self {
            name: handle.name().to_string(),
            status: handle.status_snapshot(),
            hostname: ssh::hostname_of(handle),
            workspace_path: workspace_path_of(handle),
        }
    }
}

/// Returns the guest path a sandbox mounts its workspace at, which is its working directory.
fn workspace_path_of(handle: &SandboxHandle) -> Option<String> {
    handle.config().ok()?.spec.runtime.workdir
}

/// Manages sandboxes and the secrets they get.
pub struct SandboxManager {
    secrets: SecretStore,
    // Held while secrets are stored or added to sandboxes, so a sandbox that is being created
    // can't miss a secret that is being set, and concurrent sets can't mix up values.
    secrets_lock: tokio::sync::Mutex<()>,
}

impl SandboxManager {
    /// Creates a manager that adds the secrets from `secrets` to sandboxes.
    pub fn new(secrets: SecretStore) -> Self {
        Self {
            secrets,
            secrets_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Starts an existing sandbox or creates a new one when it doesn't exist, installs the
    /// workspace's mise tools when it started, then syncs the SSH config and the editor
    /// settings, also when starting failed. `resources` is only called when the sandbox is
    /// created.
    pub async fn start(
        &self,
        request: StartSandbox<'_>,
        resources: impl FnOnce() -> Result<Resources, SandboxError>,
    ) -> Result<(), SandboxError> {
        let result = match Sandbox::get(request.name).await {
            Ok(existing_sb) => start_existing_sandbox(&existing_sb, request).await,
            Err(_) => self.create_and_install(request, resources).await,
        };

        sync_ssh_config().await;

        result
    }

    /// Stops a running sandbox, killing it when it doesn't shut down within [`STOP_TIMEOUT`].
    pub async fn stop(&self, name: &str) -> Result<(), SandboxError> {
        stop_or_kill(&get_sandbox(name).await?, STOP_TIMEOUT)
            .await
            .map_err(|err| {
                tracing::error!(error = ?err, "failed to stop sandbox {name}");
                SandboxError::internal("failed to stop sandbox")
            })
    }

    /// Removes a sandbox, then syncs the SSH config and the editor settings. A running sandbox
    /// is only removed with `force`, which stops it first like [`SandboxManager::stop`].
    pub async fn remove(&self, name: &str, force: bool) -> Result<(), SandboxError> {
        let sb = get_sandbox(name).await?;

        if force && is_live(sb.status_snapshot()) {
            stop_or_kill(&sb, STOP_TIMEOUT)
                .await
                .map_err(|err| stop_before_remove_failed(name, &err))?;
        }

        sb.remove().await.map_err(|err| match err {
            MicrosandboxError::SandboxStillRunning(_) => SandboxError::failed_precondition(
                format!("sandbox {name} is running; stop it first or remove it with force"),
            ),
            err => {
                tracing::error!(error = ?err, "failed to remove sandbox {name}");
                SandboxError::internal("failed to remove sandbox")
            }
        })?;

        sync_ssh_config().await;

        Ok(())
    }

    /// Returns the sandbox with the name.
    pub async fn get(&self, name: &str) -> Result<SandboxInfo, SandboxError> {
        Ok(SandboxInfo::of(&get_sandbox(name).await?))
    }

    /// Returns every sandbox.
    pub async fn list(&self) -> Result<Vec<SandboxInfo>, SandboxError> {
        Ok(list_all_sandboxes()
            .await?
            .iter()
            .map(SandboxInfo::of)
            .collect())
    }

    /// Connects to the running sandbox with the name.
    pub async fn connect(&self, name: &str) -> Result<Sandbox, SandboxError> {
        get_sandbox(name)
            .await?
            .connect()
            .await
            .map_err(|err| connect_failed(name, &err))
    }

    /// Connects to the sandbox with the SSH host name, starting it when needed.
    pub async fn connect_by_hostname(&self, hostname: &str) -> Result<Sandbox, SandboxError> {
        Sandbox::list_with(|opt| opt.label(ssh::HOSTNAME_LABEL, hostname))
            .await
            .map_err(|err| {
                tracing::error!(error = ?err, "failed to list sandboxes with host name {hostname}");
                SandboxError::internal("failed to list sandboxes")
            })?
            .sandboxes
            .into_iter()
            .next()
            .ok_or_else(|| SandboxError::not_found("couldn't find a sandbox with that host name"))?
            .connect_or_start_detached()
            .await
            .map_err(|err| {
                tracing::error!(error = ?err, "failed to start sandbox with host name {hostname}");
                SandboxError::failed_precondition("failed to start sandbox")
            })
    }

    /// Stores a secret and adds it to the existing sandboxes. Running sandboxes pick it up the
    /// next time they start. Returns the names of the sandboxes it couldn't be added to.
    pub async fn set_secret(
        &self,
        name: String,
        value: String,
        allowed_hosts: Vec<String>,
    ) -> Result<Vec<String>, SandboxError> {
        let secret = Secret::new(name, value, allowed_hosts)
            .map_err(|err| SandboxError::invalid_argument(err.to_string()))?;

        let _secrets_guard = self.secrets_lock.lock().await;

        self.secrets.set(secret.clone()).map_err(|err| {
            tracing::error!(error = ?err, "failed to store secret {}", secret.name());
            SandboxError::internal("failed to store secret")
        })?;

        let failed_sandboxes = update_firebrick_sandboxes(SecretChange::Add(&secret))
            .await
            .map_err(|_| {
                SandboxError::internal(
                    "stored the secret, but failed to list the sandboxes to add it to",
                )
            })?;

        tracing::info!("set secret {}", secret.name());

        Ok(failed_sandboxes)
    }

    /// Returns the stored secrets, sorted by name.
    pub fn list_secrets(&self) -> Result<Vec<Secret>, SandboxError> {
        let mut secrets = self.load_secrets()?;
        secrets.sort_by(|a, b| a.name().cmp(b.name()));
        Ok(secrets)
    }

    /// Removes a stored secret and removes it from the existing sandboxes. Running sandboxes
    /// keep it until they restart. Returns the names of the sandboxes it couldn't be removed
    /// from, in which case the secret stays in the store so the removal can be retried.
    pub async fn remove_secret(&self, name: &str) -> Result<Vec<String>, SandboxError> {
        secrets::validate_name(name)
            .map_err(|err| SandboxError::invalid_argument(err.to_string()))?;

        let _secrets_guard = self.secrets_lock.lock().await;
        self.ensure_secret_exists(name)?;

        // Remove the secret from the store last, so it can be removed again when a sandbox
        // fails.
        let failed_sandboxes = update_firebrick_sandboxes(SecretChange::Remove(name)).await?;

        if !failed_sandboxes.is_empty() {
            return Ok(failed_sandboxes);
        }

        self.secrets.remove(name).map_err(|err| {
            tracing::error!(error = ?err, "failed to remove secret {name}");
            SandboxError::internal("failed to remove secret")
        })?;

        tracing::info!("removed secret {name}");

        Ok(failed_sandboxes)
    }

    /// Returns the stored secrets.
    fn load_secrets(&self) -> Result<Vec<Secret>, SandboxError> {
        self.secrets.load().map_err(|err| {
            tracing::error!(error = ?err, "failed to load secrets");
            SandboxError::internal("failed to load secrets")
        })
    }

    /// Fails with `NotFound` when no secret with the name is stored.
    fn ensure_secret_exists(&self, name: &str) -> Result<(), SandboxError> {
        if self
            .load_secrets()?
            .iter()
            .any(|secret| secret.name() == name)
        {
            Ok(())
        } else {
            Err(SandboxError::not_found(format!(
                "secret {name} doesn't exist"
            )))
        }
    }

    /// Creates a sandbox, then installs its workspace's mise tools when the request enables
    /// mise.
    async fn create_and_install(
        &self,
        request: StartSandbox<'_>,
        resources: impl FnOnce() -> Result<Resources, SandboxError>,
    ) -> Result<(), SandboxError> {
        let (sb, guest_path) = self.create_sandbox(request, resources).await?;

        if request.mise {
            install_mise_tools(&sb, &guest_path).await?;
        }

        Ok(())
    }

    /// Creates a sandbox for the workspace with the stored secrets. Returns it with the guest
    /// path of its workspace.
    async fn create_sandbox(
        &self,
        request: StartSandbox<'_>,
        resources: impl FnOnce() -> Result<Resources, SandboxError>,
    ) -> Result<(Sandbox, String), SandboxError> {
        let guest_path = workspace_mount_path(request.workspace)?;
        let resources = resources()?;
        let hostname = ssh::pick_hostname(request.workspace, &taken_hostnames().await?);
        let _secrets_guard = self.secrets_lock.lock().await;
        let secrets = self.load_secrets()?;

        let builder = Sandbox::builder(request.name)
            .image(sandbox_image(request.image))
            .label(ssh::HOSTNAME_LABEL, &hostname)
            .label(mise::ENABLED_LABEL, mise::label_value(request.mise))
            // Mounted read/write; Mirror propagates guest chmod changes to the host files.
            // Mount has the ownership in the guest set to the 1000/1000 (agent) user.
            .volume(&guest_path, |m| {
                m.bind(request.workspace)
                    .host_permissions(HostPermissions::Mirror)
                    .owner(1000, 1000)
            })
            .workdir(&guest_path)
            .detached(true);
        let builder = with_init(with_resources(builder, resources), request.init);

        let sb = match create_with(builder, &secrets, request.network).await {
            Ok(sb) => sb,
            Err(err) => {
                tracing::error!(error = ?err, "failed to create sandbox {}", request.name);
                return Err(create_failed(request.name, &err.to_string()).await);
            }
        };

        tracing::info!("created sandbox {} as {hostname}", request.name);

        Ok((sb, guest_path))
    }
}

/// Adds the egress rules and the secrets to the builder and creates the sandbox. The rules go
/// first, because they replace the TLS settings that the secrets turn on.
async fn create_with(
    builder: SandboxBuilder,
    secrets: &[Secret],
    network: &NetworkSpec,
) -> Result<Sandbox, MicrosandboxError> {
    let builder = network::add_to_builder(builder, network)?;

    secrets::add_to_builder(builder, secrets).create().await
}

/// Returns the sandbox with the name.
async fn get_sandbox(name: &str) -> Result<SandboxHandle, SandboxError> {
    Sandbox::get(name).await.map_err(|err| match err {
        MicrosandboxError::SandboxNotFound(_) => {
            SandboxError::not_found("couldn't find specified sandbox")
        }
        err => {
            tracing::error!(error = ?err, "failed to get sandbox {name}");
            SandboxError::internal("failed to get sandbox")
        }
    })
}

/// Logs why connecting to a sandbox failed and returns the error the client sees. A stopped
/// sandbox is a normal situation, so only other failures are logged as errors.
fn connect_failed(name: &str, err: &MicrosandboxError) -> SandboxError {
    if matches!(err, MicrosandboxError::SandboxNotRunning(_)) {
        tracing::warn!(error = ?err, "failed to connect to sandbox {name}");
    } else {
        tracing::error!(error = ?err, "failed to connect to sandbox {name}");
    }

    SandboxError::failed_precondition("sandbox is not running")
}

/// Logs why a forced remove couldn't stop the sandbox and returns the error the client sees.
fn stop_before_remove_failed(name: &str, err: &MicrosandboxError) -> SandboxError {
    tracing::error!(error = ?err, "failed to stop sandbox {name} before removing it");
    SandboxError::internal("failed to stop sandbox before removing it")
}

/// Stops the sandbox gracefully, and kills it when it hasn't stopped within `timeout`.
pub async fn stop_or_kill(sb: &SandboxHandle, timeout: Duration) -> Result<(), MicrosandboxError> {
    match sb.stop_with_timeout(timeout).await {
        Err(MicrosandboxError::StopTimeout { .. }) => {
            tracing::warn!(
                "sandbox {} didn't stop within {timeout:?}; killing it",
                sb.name()
            );
            sb.kill().await
        }
        result => result,
    }
}

/// Whether microsandbox refuses to remove a sandbox with the status because it's still running.
fn is_live(status: SandboxStatus) -> bool {
    matches!(
        status,
        SandboxStatus::Starting
            | SandboxStatus::Running
            | SandboxStatus::Draining
            | SandboxStatus::Paused
    )
}

/// Starts an existing sandbox, first giving it a host name when it has none, then installs its
/// workspace's mise tools when the sandbox has mise enabled. Does nothing when the sandbox is
/// already running or starting.
async fn start_existing_sandbox(
    sb: &SandboxHandle,
    request: StartSandbox<'_>,
) -> Result<(), SandboxError> {
    // Sandboxes created before SSH support have no host name yet.
    if ssh::hostname_of(sb).is_none() {
        assign_hostname(sb, request).await?;
    }

    if matches!(
        sb.status_snapshot(),
        SandboxStatus::Running | SandboxStatus::Starting
    ) {
        return Ok(());
    }

    let started = match sb.start_detached().await {
        Ok(started) => started,
        // Another request started the sandbox after its status was read, and installs the tools.
        Err(MicrosandboxError::SandboxStillRunning(_)) => return Ok(()),
        Err(err) => {
            tracing::error!(error = ?err, "failed to start sandbox {}", request.name);
            return Err(SandboxError::internal("failed to start sandbox"));
        }
    };

    tracing::info!("started sandbox {}", request.name);

    match workspace_path_of(sb) {
        Some(workspace) if mise_enabled(sb) => install_mise_tools(&started, &workspace).await,
        _ => Ok(()),
    }
}

/// Whether the sandbox installs its workspace's mise tools on start.
fn mise_enabled(sb: &SandboxHandle) -> bool {
    sb.config()
        .is_ok_and(|config| mise::is_enabled(&config.spec.labels))
}

/// Trusts and installs the mise tools of the workspace. An image without mise only logs a
/// warning, because mise is a convenience the developer of a custom image may leave out.
async fn install_mise_tools(sb: &Sandbox, workspace: &str) -> Result<(), SandboxError> {
    match mise::install_tools(sb, workspace).await {
        Ok(()) => Ok(()),
        Err(err @ MiseError::Failed { .. }) => {
            tracing::warn!("{err}");
            Err(SandboxError::failed_precondition(err.to_string()))
        }
        Err(err @ MiseError::NotInstalled(_)) => {
            tracing::warn!("skipped installing mise tools: {err}");
            Ok(())
        }
        Err(err @ MiseError::Exec { .. }) => {
            tracing::error!(error = ?err, "failed to install mise tools in {}", sb.name());
            Err(SandboxError::internal("failed to install mise tools"))
        }
    }
}

/// Gives the sandbox a host name based on its workspace, logging a warning when that fails.
/// Fails when the host names other sandboxes use can't be listed.
async fn assign_hostname(
    sb: &SandboxHandle,
    request: StartSandbox<'_>,
) -> Result<(), SandboxError> {
    let workspace = match request.workspace {
        "" => request.name,
        workspace => workspace,
    };
    let hostname = ssh::pick_hostname(workspace, &taken_hostnames().await?);

    let result = sb
        .modify()
        .label(ssh::HOSTNAME_LABEL, &hostname)
        .apply()
        .await;

    if let Err(err) = result {
        tracing::warn!("failed to assign host name to {}: {err}", request.name);
    }

    Ok(())
}

/// Returns every sandbox, following the list cursor across pages.
async fn list_all_sandboxes() -> Result<Vec<SandboxHandle>, SandboxError> {
    let mut sandboxes = vec![];
    let mut next_cursor: Option<String> = None;

    loop {
        let result = Sandbox::list_with(|opt| match next_cursor.take() {
            Some(cursor) => opt.cursor(cursor),
            None => opt,
        })
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, "failed to list sandboxes");
            SandboxError::internal("failed to list sandboxes")
        })?;

        sandboxes.extend(result.sandboxes);

        if result.next_cursor.is_none() {
            break;
        }

        next_cursor = result.next_cursor;
    }

    Ok(sandboxes)
}

/// A change to the secrets of existing sandboxes.
enum SecretChange<'a> {
    Add(&'a Secret),
    Remove(&'a str),
}

impl fmt::Display for SecretChange<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretChange::Add(secret) => write!(f, "add secret {} to", secret.name()),
            SecretChange::Remove(name) => write!(f, "remove secret {name} from"),
        }
    }
}

/// Applies a secret change to every sandbox firebrick created, which are the ones with a host
/// name. Returns the names of the sandboxes the change failed for.
async fn update_firebrick_sandboxes(change: SecretChange<'_>) -> Result<Vec<String>, SandboxError> {
    let mut failed = vec![];

    for handle in list_all_sandboxes().await? {
        if ssh::hostname_of(&handle).is_none() {
            continue;
        }

        let result = match change {
            SecretChange::Add(secret) => secrets::add_to_sandbox(&handle, secret).await,
            SecretChange::Remove(name) => secrets::remove_from_sandbox(&handle, name).await,
        };

        if let Err(err) = result {
            tracing::warn!(error = ?err, "failed to {change} sandbox {}", handle.name());
            failed.push(handle.name().to_string());
        }
    }

    Ok(failed)
}

/// Returns the SSH host names that sandboxes already use.
async fn taken_hostnames() -> Result<HashSet<String>, SandboxError> {
    Ok(list_all_sandboxes()
        .await?
        .iter()
        .filter_map(ssh::hostname_of)
        .collect())
}

/// Regenerates the SSH config, the editors' Remote-SSH settings and Zed's remote projects from
/// the current sandboxes, logging a warning when that fails. SSH access is a convenience, so a
/// failure here doesn't fail the request.
pub async fn sync_ssh_config() {
    let sandboxes = match list_all_sandboxes().await {
        Ok(sandboxes) => sandboxes,
        Err(err) => {
            tracing::warn!("failed to update SSH config: {err}");
            return;
        }
    };
    let hostnames: Vec<String> = sandboxes.iter().filter_map(ssh::hostname_of).collect();

    if let Err(err) = ssh::sync_config(&hostnames) {
        tracing::warn!("failed to update SSH config: {err:#}");
    }

    sync_editor_settings(&hostnames);
    sync_zed_settings(&sandboxes);
}

/// Syncs the host names into the editors' Remote-SSH settings, logging a warning per editor
/// that fails.
fn sync_editor_settings(hostnames: &[String]) {
    let Some(config_root) = vscode::config_root() else {
        tracing::warn!("failed to update editor settings: HOME is not set");
        return;
    };

    for err in vscode::sync_settings(&config_root, hostnames) {
        tracing::warn!(
            "failed to update editor settings: {:#}",
            anyhow::Error::from(err)
        );
    }
}

/// Syncs the sandboxes with a host name into Zed's remote projects, logging a warning when
/// that fails.
fn sync_zed_settings(sandboxes: &[SandboxHandle]) {
    let Some(config_root) = zed::config_root() else {
        tracing::warn!("failed to update Zed settings: HOME is not set");
        return;
    };
    let projects: Vec<zed::RemoteProject> = sandboxes.iter().filter_map(remote_project).collect();

    if let Err(err) = zed::sync_settings(&config_root, &projects) {
        tracing::warn!(
            "failed to update Zed settings: {:#}",
            anyhow::Error::from(err)
        );
    }
}

/// Returns the sandbox as a Zed remote project, or `None` when it has no host name.
fn remote_project(handle: &SandboxHandle) -> Option<zed::RemoteProject> {
    Some(zed::RemoteProject {
        host: ssh::hostname_of(handle)?,
        nickname: handle.name().to_string(),
        workspace_path: workspace_path_of(handle),
    })
}

/// Returns the guest path for a workspace: `/workspaces/<leaf-name>` of the absolute host path.
fn workspace_mount_path(workspace: &str) -> Result<String, SandboxError> {
    let path = Path::new(workspace);

    if !path.is_absolute() {
        return Err(SandboxError::invalid_argument(
            "workspace must be an absolute path",
        ));
    }

    let leaf = path.file_name().ok_or_else(|| {
        SandboxError::invalid_argument("workspace must not be the root directory")
    })?;

    Ok(format!("/workspaces/{}", leaf.to_string_lossy()))
}

/// Returns the requested image, or the default image when the request doesn't name one.
fn sandbox_image(image: &str) -> &str {
    if image.is_empty() {
        firebrick_spec::DEFAULT_IMAGE
    } else {
        image
    }
}

/// Path of the init that runs as PID 1 in sandboxes with `init` enabled.
const INIT_PATH: &str = "/sbin/init";

/// Gives the sandbox its vCPUs, memory and Docker volume.
fn with_resources(builder: SandboxBuilder, resources: Resources) -> SandboxBuilder {
    builder
        .cpus(resources.cpus)
        .memory(resources.memory_mib)
        // A private ext4 disk, because Docker's overlayfs storage can't sit on the overlayfs
        // root. It lives until the sandbox is removed.
        .volume(DOCKER_DATA_PATH, |m| {
            m.owned_with(|v| v.disk().size(resources.docker_volume_mib))
        })
}

/// Hands PID 1 to the image's `/sbin/init` when `init` is enabled.
fn with_init(builder: SandboxBuilder, init: bool) -> SandboxBuilder {
    if init {
        builder.init(INIT_PATH)
    } else {
        builder
    }
}

/// Returns the error for a failed create. A sandbox whose init failed to boot is removed, so
/// a start with `init: false` can create it again.
async fn create_failed(name: &str, message: &str) -> SandboxError {
    let error = create_error(message);
    if matches!(error, SandboxError::FailedPrecondition(_))
        && let Err(err) = Sandbox::remove(name).await
    {
        tracing::warn!("failed to remove sandbox {name} after a failed create: {err}");
    }

    error
}

/// Turns the message of a failed create into the error the client sees.
///
/// microsandbox only reports a missing init as text, so this matches on it; other failures
/// stay internal.
fn create_error(message: &str) -> SandboxError {
    if message.contains("handoff failed") {
        SandboxError::failed_precondition(
            "failed to create sandbox: the image has no /sbin/init; add one or set init: false \
             in .firebrick.yml",
        )
    } else {
        SandboxError::internal("failed to create sandbox")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns a manager with a store at `path` that has a secret for `example.com` under each
    /// name.
    fn manager_with_secrets(path: &Path, names: &[&str]) -> SandboxManager {
        let store = SecretStore::new(path);

        for name in names {
            let secret =
                Secret::new(name.to_string(), "value".into(), vec!["example.com".into()]).unwrap();
            store.set(secret).unwrap();
        }

        SandboxManager::new(store)
    }

    #[test]
    fn sandbox_image_falls_back_to_default() {
        assert_eq!(sandbox_image("alpine:3.22"), "alpine:3.22");
        assert_eq!(sandbox_image(""), firebrick_spec::DEFAULT_IMAGE);
    }

    #[test]
    fn create_error_explains_missing_init() {
        let error = create_error(
            "guest initialization failed: handoff failed: init error: no init binary found",
        );

        assert!(
            matches!(&error, SandboxError::FailedPrecondition(message) if message.contains("set init: false")),
            "{error:?}"
        );
    }

    #[test]
    fn create_error_hides_other_failures() {
        assert_eq!(
            create_error("image pull failed"),
            SandboxError::internal("failed to create sandbox")
        );
    }

    #[test]
    fn workspace_mount_path_uses_leaf_name() {
        assert_eq!(
            workspace_mount_path("/home/user/project").unwrap(),
            "/workspaces/project"
        );
        assert_eq!(
            workspace_mount_path("/home/user/project/").unwrap(),
            "/workspaces/project"
        );
    }

    #[test]
    fn workspace_mount_path_rejects_relative_empty_and_root_paths() {
        for workspace in ["", "project", "./project", "/"] {
            assert!(
                matches!(
                    workspace_mount_path(workspace),
                    Err(SandboxError::InvalidArgument(_))
                ),
                "{workspace:?} should be rejected"
            );
        }
    }

    #[test]
    fn list_secrets_sorts_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager_with_secrets(&dir.path().join("secrets.yml"), &["B", "A"]);

        let names: Vec<String> = manager
            .list_secrets()
            .unwrap()
            .iter()
            .map(|secret| secret.name().to_string())
            .collect();

        assert_eq!(names, ["A", "B"]);
    }

    #[tokio::test]
    async fn set_secret_rejects_invalid_secret_without_storing_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.yml");
        let manager = SandboxManager::new(SecretStore::new(&path));

        let result = manager
            .set_secret("NOT-A-NAME".into(), "value".into(), vec![])
            .await;

        assert!(matches!(result, Err(SandboxError::InvalidArgument(_))));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn remove_secret_returns_not_found_and_keeps_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.yml");
        let manager = manager_with_secrets(&path, &["A"]);

        let result = manager.remove_secret("B").await;

        assert_eq!(
            result,
            Err(SandboxError::NotFound("secret B doesn't exist".into()))
        );
        assert_eq!(SecretStore::new(&path).load().unwrap().len(), 1);
    }
}

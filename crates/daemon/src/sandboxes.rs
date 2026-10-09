//! Sandbox management on top of microsandbox: the sandbox lifecycle, the secrets of
//! sandboxes, SSH host names and the generated SSH config.

use crate::secrets::{self, Secret, SecretStore};
use crate::ssh;
use microsandbox::sandbox::{HostPermissions, SandboxBuilder, SandboxHandle, SandboxStatus};
use microsandbox::{MicrosandboxError, Sandbox};
use std::collections::HashSet;
use std::fmt;
use std::path::Path;
use thiserror::Error;

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
}

/// The name, status and SSH host name of a sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxInfo {
    /// Name of the sandbox.
    pub name: String,
    /// Status of the sandbox.
    pub status: SandboxStatus,
    /// SSH host name of the sandbox, if it has one.
    pub hostname: Option<String>,
}

impl SandboxInfo {
    fn of(handle: &SandboxHandle) -> Self {
        Self {
            name: handle.name().to_string(),
            status: handle.status_snapshot(),
            hostname: ssh::hostname_of(handle),
        }
    }
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

    /// Starts an existing sandbox or creates a new one when it doesn't exist, then syncs the
    /// SSH config. `resources` is only called when the sandbox is created.
    pub async fn start(
        &self,
        request: StartSandbox<'_>,
        resources: impl FnOnce() -> Result<Resources, SandboxError>,
    ) -> Result<(), SandboxError> {
        match Sandbox::get(request.name).await {
            Ok(existing_sb) => start_existing_sandbox(&existing_sb, request).await?,
            Err(_) => self.create_sandbox(request, resources).await?,
        }

        sync_ssh_config().await;

        Ok(())
    }

    /// Stops a running sandbox.
    pub async fn stop(&self, name: &str) -> Result<(), SandboxError> {
        get_sandbox(name)
            .await?
            .stop()
            .await
            .map_err(|_| SandboxError::internal("failed to stop sandbox"))
    }

    /// Removes a sandbox, then syncs the SSH config.
    pub async fn remove(&self, name: &str) -> Result<(), SandboxError> {
        get_sandbox(name)
            .await?
            .remove()
            .await
            .map_err(|_| SandboxError::internal("failed to remove sandbox"))?;

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
            .map_err(|_| SandboxError::failed_precondition("sandbox is not running"))
    }

    /// Connects to the sandbox with the SSH host name, starting it when needed.
    pub async fn connect_by_hostname(&self, hostname: &str) -> Result<Sandbox, SandboxError> {
        Sandbox::list_with(|opt| opt.label(ssh::HOSTNAME_LABEL, hostname))
            .await
            .map_err(|_| SandboxError::internal("failed to list sandboxes"))?
            .sandboxes
            .into_iter()
            .next()
            .ok_or_else(|| SandboxError::not_found("couldn't find a sandbox with that host name"))?
            .connect_or_start_detached()
            .await
            .map_err(|_| SandboxError::failed_precondition("failed to start sandbox"))
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
            tracing::warn!("failed to store secret {}: {err:#}", secret.name());
            SandboxError::internal("failed to store secret")
        })?;

        let failed_sandboxes = update_anvil_sandboxes(SecretChange::Add(&secret))
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
        let failed_sandboxes = update_anvil_sandboxes(SecretChange::Remove(name)).await?;

        if !failed_sandboxes.is_empty() {
            return Ok(failed_sandboxes);
        }

        self.secrets.remove(name).map_err(|err| {
            tracing::warn!("failed to remove secret {name}: {err:#}");
            SandboxError::internal("failed to remove secret")
        })?;

        tracing::info!("removed secret {name}");

        Ok(failed_sandboxes)
    }

    /// Returns the stored secrets.
    fn load_secrets(&self) -> Result<Vec<Secret>, SandboxError> {
        self.secrets.load().map_err(|err| {
            tracing::warn!("failed to load secrets: {err:#}");
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

    /// Creates a sandbox for the workspace with the stored secrets.
    async fn create_sandbox(
        &self,
        request: StartSandbox<'_>,
        resources: impl FnOnce() -> Result<Resources, SandboxError>,
    ) -> Result<(), SandboxError> {
        let guest_path = workspace_mount_path(request.workspace)?;
        let resources = resources()?;
        let hostname = ssh::pick_hostname(request.workspace, &taken_hostnames().await?);
        let _secrets_guard = self.secrets_lock.lock().await;
        let secrets = self.load_secrets()?;

        let builder = Sandbox::builder(request.name)
            .image(sandbox_image(request.image))
            .cpus(resources.cpus)
            .memory(resources.memory_mib)
            .label(ssh::HOSTNAME_LABEL, &hostname)
            // Mounted read/write; Mirror propagates guest chmod changes to the host files.
            // Mount has the ownership in the guest set to the 1000/1000 (agent) user.
            .volume(&guest_path, |m| {
                m.bind(request.workspace)
                    .host_permissions(HostPermissions::Mirror)
                    .owner(1000, 1000)
            })
            .workdir(&guest_path)
            .detached(true);

        if let Err(err) = secrets::add_to_builder(with_init(builder, request.init), &secrets)
            .create()
            .await
        {
            tracing::warn!("failed to create sandbox {}: {err}", request.name);
            return Err(create_failed(request.name, &err.to_string()).await);
        }

        tracing::info!("created sandbox {} as {hostname}", request.name);

        Ok(())
    }
}

/// Returns the sandbox with the name.
async fn get_sandbox(name: &str) -> Result<SandboxHandle, SandboxError> {
    Sandbox::get(name).await.map_err(|err| match err {
        MicrosandboxError::SandboxNotFound(_) => {
            SandboxError::not_found("couldn't find specified sandbox")
        }
        _ => SandboxError::internal("failed to get sandbox"),
    })
}

/// Starts an existing sandbox, first giving it a host name when it has none. Does nothing when
/// the sandbox is already running or starting.
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

    match sb.start_detached().await {
        Ok(_) => tracing::info!("started sandbox {}", request.name),
        // Another request started the sandbox after its status was read.
        Err(MicrosandboxError::SandboxStillRunning(_)) => {}
        Err(err) => {
            tracing::warn!("failed to start sandbox {}: {err}", request.name);
            return Err(SandboxError::internal("failed to start sandbox"));
        }
    }

    Ok(())
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
        .map_err(|_| SandboxError::internal("failed to list sandboxes"))?;

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

/// Applies a secret change to every sandbox anvil created, which are the ones with a host
/// name. Returns the names of the sandboxes the change failed for.
async fn update_anvil_sandboxes(change: SecretChange<'_>) -> Result<Vec<String>, SandboxError> {
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
            tracing::warn!("failed to {change} sandbox {}: {err}", handle.name());
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

/// Regenerates the SSH config from the current sandboxes, logging a warning when that fails.
/// SSH access is a convenience, so a failure here doesn't fail the request.
pub async fn sync_ssh_config() {
    let hostnames: Vec<String> = match list_all_sandboxes().await {
        Ok(sandboxes) => sandboxes.iter().filter_map(ssh::hostname_of).collect(),
        Err(err) => {
            tracing::warn!("failed to update SSH config: {err}");
            return;
        }
    };

    if let Err(err) = ssh::sync_config(&hostnames) {
        tracing::warn!("failed to update SSH config: {err:#}");
    }
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
        anvil_spec::DEFAULT_IMAGE
    } else {
        image
    }
}

/// Path of the init that runs as PID 1 in sandboxes with `init` enabled.
const INIT_PATH: &str = "/sbin/init";

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
             in .anvil.yml",
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
        assert_eq!(sandbox_image(""), anvil_spec::DEFAULT_IMAGE);
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

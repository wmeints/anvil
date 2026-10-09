use anvil_spec::SandboxSpec;
use anyhow::{Result, anyhow, bail};
use clap::ValueEnum;
use serde::Serialize;
use std::ops::ControlFlow;
use std::path::Path;
use std::time::Duration;
use tokio::time::{Instant, sleep};
use tonic::Code;
use tonic::transport::Channel;

use crate::api::{
    GetSandboxRequest, GetSandboxResponse, ListSandboxesRequest, ListSandboxesResponse,
    RemoveSandboxRequest, SandboxResources, SandboxStatus, SandboxSummary, StartSandboxRequest,
    StopSandboxRequest, sandbox_management_service_client::SandboxManagementServiceClient,
};
use crate::table;

pub(crate) const SPEC_FILE_NAME: &str = ".anvil.yml";

/// Starts the named sandbox, or the sandbox for the working directory without a name. Does
/// nothing when it's already running, and waits for it when it's starting. Only the sandbox for
/// the working directory is created when it doesn't exist, because creating one needs its spec.
pub async fn start_sandbox(
    name: Option<String>,
    working_dir: &Path,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<()> {
    let name = match name {
        Some(name) => start_named_sandbox(name, client).await?,
        None => start_working_dir_sandbox(working_dir, client).await?,
    };

    let sandbox = client
        .get_sandbox(GetSandboxRequest { name })
        .await?
        .into_inner();

    print!("{}", connect_instructions(&sandbox));

    Ok(())
}

/// Returns the commands that connect to the sandbox over SSH and open its workspace in VS Code
/// and Zed, one per line. The editor commands need both the host name and the workspace path.
fn connect_instructions(sandbox: &GetSandboxResponse) -> String {
    let (hostname, workspace_path) = (&sandbox.hostname, &sandbox.workspace_path);

    match (hostname.is_empty(), workspace_path.is_empty()) {
        (true, _) => String::new(),
        (false, true) => format!("Connect with: ssh {hostname}\n"),
        (false, false) => format!(
            "Connect with: ssh {hostname}\n\
             Open in VS Code: code --folder-uri vscode-remote://ssh-remote+{hostname}{workspace_path}\n\
             Open in Zed: zed ssh://{hostname}{workspace_path}\n"
        ),
    }
}

/// Starts the existing sandbox with the name and returns the name.
async fn start_named_sandbox(
    name: String,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<String> {
    // The daemon falls back to the sandbox name for the host name when the workspace is empty.
    if !start_if_exists(&name, Path::new(""), client).await? {
        bail!(
            "sandbox {name} doesn't exist; run anvil start in its project directory to create it"
        );
    }

    Ok(name)
}

/// Starts the sandbox for the working directory, creating it when needed, and returns its name.
async fn start_working_dir_sandbox(
    working_dir: &Path,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<String> {
    let spec = resolve_spec(working_dir)?;
    let name = spec.name.clone();

    ensure_running(spec, working_dir, client).await?;

    Ok(name)
}

const STARTING_TIMEOUT: Duration = Duration::from_secs(120);
const STARTING_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Makes sure the sandbox exists and is running, creating or starting it when needed.
pub(crate) async fn ensure_running(
    spec: SandboxSpec,
    workspace: &Path,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<()> {
    if start_if_exists(&spec.name, workspace, client).await? {
        return Ok(());
    }

    eprintln!("Creating sandbox {}...", spec.name);
    client
        .start_sandbox(build_start_request(spec, workspace))
        .await?;

    Ok(())
}

/// Starts the sandbox when it exists, waiting while it's starting. Returns `false` when the
/// sandbox doesn't exist.
async fn start_if_exists(
    name: &str,
    workspace: &Path,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<bool> {
    let deadline = Instant::now() + STARTING_TIMEOUT;

    loop {
        let Some(status) = sandbox_status(client, name).await? else {
            return Ok(false);
        };

        if start_existing_sandbox(name, status, workspace, client)
            .await?
            .is_break()
        {
            return Ok(true);
        }

        if Instant::now() >= deadline {
            bail!("timed out waiting for sandbox {name} to start");
        }

        sleep(STARTING_POLL_INTERVAL).await;
    }
}

/// Returns the status of the sandbox, or `None` when it doesn't exist.
async fn sandbox_status(
    client: &mut SandboxManagementServiceClient<Channel>,
    name: &str,
) -> Result<Option<SandboxStatus>> {
    let request = GetSandboxRequest {
        name: name.to_string(),
    };

    match client.get_sandbox(request).await {
        Ok(response) => Ok(Some(response.into_inner().status())),
        Err(status) if status.code() == Code::NotFound => Ok(None),
        Err(status) => Err(status.into()),
    }
}

/// Starts an existing sandbox when it's stopped. Returns `Continue` while the sandbox is still
/// starting, and fails when it can't be started now.
async fn start_existing_sandbox(
    name: &str,
    status: SandboxStatus,
    workspace: &Path,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<ControlFlow<()>> {
    match status {
        SandboxStatus::Running => Ok(ControlFlow::Break(())),
        SandboxStatus::Starting => Ok(ControlFlow::Continue(())),
        SandboxStatus::Stopped | SandboxStatus::Crashed => {
            eprintln!("Starting sandbox {name}...");

            // The workspace lets the daemon name sandboxes created before SSH support.
            client
                .start_sandbox(StartSandboxRequest {
                    name: name.to_string(),
                    workspace: workspace.to_string_lossy().into_owned(),
                    ..Default::default()
                })
                .await?;

            Ok(ControlFlow::Break(()))
        }
        status @ (SandboxStatus::Stopping | SandboxStatus::Paused) => {
            bail!(
                "sandbox {name} is {}; try again once it has stopped",
                format_status(status).to_lowercase()
            )
        }
    }
}

/// Builds a start request from the spec, filling in the default image, resources and init.
/// The workspace is mounted into the sandbox when it's created.
fn build_start_request(spec: SandboxSpec, workspace: &Path) -> StartSandboxRequest {
    let resources = spec.resources.unwrap_or_default();

    StartSandboxRequest {
        name: spec.name,
        image: spec
            .image
            .unwrap_or_else(|| anvil_spec::DEFAULT_IMAGE.to_string()),
        resources: Some(SandboxResources {
            cpu: resources.cpu.into(),
            memory: resources.memory,
        }),
        workspace: workspace.to_string_lossy().into_owned(),
        init: Some(spec.init.unwrap_or(true)),
    }
}

/// Stops the named sandbox, or the sandbox for the working directory without a name.
pub async fn stop_sandbox(
    name: Option<String>,
    working_dir: &Path,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<()> {
    let name = sandbox_name(name, working_dir)?;

    client
        .stop_sandbox(StopSandboxRequest { name: name.clone() })
        .await
        .map_err(|status| describe_status(&name, status))?;

    Ok(())
}

/// Removes the named sandbox, or the sandbox for the working directory without a name.
pub async fn remove_sandbox(
    name: Option<String>,
    working_dir: &Path,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<()> {
    let name = sandbox_name(name, working_dir)?;

    client
        .remove_sandbox(RemoveSandboxRequest { name: name.clone() })
        .await
        .map_err(|status| describe_status(&name, status))?;

    Ok(())
}

/// Returns the explicit sandbox name, or the name of the sandbox for the working directory.
/// The spec file is only read without an explicit name.
fn sandbox_name(name: Option<String>, working_dir: &Path) -> Result<String> {
    match name {
        Some(name) => Ok(name),
        None => Ok(resolve_spec(working_dir)?.name),
    }
}

/// Turns a `NotFound` status into an error that names the missing sandbox.
fn describe_status(name: &str, status: tonic::Status) -> anyhow::Error {
    if status.code() == Code::NotFound {
        return anyhow!("sandbox {name} doesn't exist");
    }

    status.into()
}

/// Output format for the list of sandboxes.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputFormat {
    /// A human-readable table.
    #[default]
    Table,
    /// A JSON array of sandboxes.
    Json,
}

/// Prints all sandboxes with their status in the given format.
pub async fn list_sandboxes(
    client: &mut SandboxManagementServiceClient<Channel>,
    format: OutputFormat,
) -> Result<()> {
    let response = client.list_sandboxes(ListSandboxesRequest {}).await?;
    let response_data: ListSandboxesResponse = response.into_inner();

    let output = match format {
        OutputFormat::Table => render_sandbox_table(&response_data.sandboxes)?,
        OutputFormat::Json => render_sandbox_json(&response_data.sandboxes)?,
    };

    print!("{output}");

    Ok(())
}

const TABLE_HEADER: [&str; 3] = ["NAME", "STATUS", "HOSTNAME"];

/// Renders the sandboxes as a table with their name, status and host name.
fn render_sandbox_table(sandboxes: &[SandboxSummary]) -> Result<String> {
    let rows: Vec<[String; 3]> = sandboxes
        .iter()
        .map(|item| {
            [
                item.name.clone(),
                format_status(item.status()),
                item.hostname.clone(),
            ]
        })
        .collect();

    table::render(TABLE_HEADER, &rows)
}

/// A sandbox as printed in the JSON output.
#[derive(Serialize)]
struct SandboxListing<'a> {
    name: &'a str,
    status: String,
    hostname: &'a str,
}

/// Renders the sandboxes as a pretty-printed JSON array.
fn render_sandbox_json(sandboxes: &[SandboxSummary]) -> Result<String> {
    let listings: Vec<SandboxListing> = sandboxes
        .iter()
        .map(|item| SandboxListing {
            name: &item.name,
            status: format_status(item.status()).to_lowercase(),
            hostname: &item.hostname,
        })
        .collect();

    Ok(format!("{}\n", serde_json::to_string_pretty(&listings)?))
}

/// Loads the sandbox spec from the working directory, falling back to a default spec
/// named after the working directory path when there's no spec file.
pub(crate) fn resolve_spec(working_dir: &Path) -> Result<SandboxSpec> {
    let spec_path = working_dir.join(SPEC_FILE_NAME);

    if spec_path.is_file() {
        return Ok(anvil_spec::from_file(&spec_path)?);
    }

    Ok(anvil_spec::default_spec(derive_name_from_path(
        working_dir,
    )?))
}

/// Encodes the full path as a sandbox name made of ASCII alphanumerics and underscores.
fn derive_name_from_path(working_dir: &Path) -> Result<String> {
    let mut name = String::new();

    for c in working_dir.to_string_lossy().chars() {
        if c.is_ascii_alphanumeric() {
            name.push(c);
        } else if !name.is_empty() && !name.ends_with('_') {
            name.push('_');
        }
    }

    let name = name.trim_end_matches('_');

    if name.is_empty() {
        bail!(
            "can't derive a sandbox name from {}; add a {SPEC_FILE_NAME} file",
            working_dir.display()
        );
    }

    Ok(name.to_string())
}

/// Returns a human-readable label for a sandbox status.
fn format_status(status: SandboxStatus) -> String {
    let status_text = match status {
        SandboxStatus::Running => "Running",
        SandboxStatus::Stopped => "Stopped",
        SandboxStatus::Starting => "Starting",
        SandboxStatus::Paused => "Paused",
        SandboxStatus::Stopping => "Stopping",
        SandboxStatus::Crashed => "Crashed",
    };

    status_text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn summary(name: &str, status: SandboxStatus, hostname: &str) -> SandboxSummary {
        SandboxSummary {
            name: name.to_string(),
            status: status.into(),
            hostname: hostname.to_string(),
        }
    }

    fn sandbox(hostname: &str, workspace_path: &str) -> GetSandboxResponse {
        GetSandboxResponse {
            name: "project".to_string(),
            hostname: hostname.to_string(),
            workspace_path: workspace_path.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn connect_instructions_include_editor_commands() {
        assert_eq!(
            connect_instructions(&sandbox("project.anvil", "/workspaces/project")),
            "Connect with: ssh project.anvil\n\
             Open in VS Code: code --folder-uri \
             vscode-remote://ssh-remote+project.anvil/workspaces/project\n\
             Open in Zed: zed ssh://project.anvil/workspaces/project\n"
        );
    }

    #[test]
    fn connect_instructions_skip_editors_without_workspace_path() {
        assert_eq!(
            connect_instructions(&sandbox("project.anvil", "")),
            "Connect with: ssh project.anvil\n"
        );
    }

    #[test]
    fn connect_instructions_are_empty_without_hostname() {
        assert_eq!(
            connect_instructions(&sandbox("", "/workspaces/project")),
            ""
        );
    }

    #[test]
    fn table_aligns_sandboxes_under_header() {
        let sandboxes = [
            summary("dev", SandboxStatus::Running, "dev.anvil"),
            summary("long_project_name", SandboxStatus::Stopped, ""),
        ];

        let table = render_sandbox_table(&sandboxes).unwrap();

        assert_eq!(
            table,
            "┌───────────────────┬─────────┬───────────┐\n\
             │ NAME              │ STATUS  │ HOSTNAME  │\n\
             ├───────────────────┼─────────┼───────────┤\n\
             │ dev               │ Running │ dev.anvil │\n\
             │ long_project_name │ Stopped │           │\n\
             └───────────────────┴─────────┴───────────┘\n"
        );
    }

    #[test]
    fn table_aligns_wide_characters() {
        let sandboxes = [summary("名前", SandboxStatus::Running, "")];

        let table = render_sandbox_table(&sandboxes).unwrap();
        let widths: Vec<usize> = table
            .lines()
            .map(|line| ratatui::text::Line::from(line).width())
            .collect();

        assert!(widths.windows(2).all(|pair| pair[0] == pair[1]), "{table}");
    }

    #[test]
    fn empty_table_shows_header() {
        let table = render_sandbox_table(&[]).unwrap();

        assert_eq!(
            table,
            "┌──────┬────────┬──────────┐\n\
             │ NAME │ STATUS │ HOSTNAME │\n\
             ├──────┼────────┼──────────┤\n\
             └──────┴────────┴──────────┘\n"
        );
    }

    #[test]
    fn json_lists_sandboxes() {
        let sandboxes = [summary("dev", SandboxStatus::Running, "dev.anvil")];

        let json = render_sandbox_json(&sandboxes).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(
            value,
            serde_json::json!([{ "name": "dev", "status": "running", "hostname": "dev.anvil" }])
        );
    }

    #[test]
    fn empty_json_is_empty_array() {
        assert_eq!(render_sandbox_json(&[]).unwrap(), "[]\n");
    }

    #[test]
    fn uses_name_from_spec_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(SPEC_FILE_NAME), "name: dev\n").unwrap();

        let spec = resolve_spec(dir.path()).unwrap();

        assert_eq!(spec.name, "dev");
    }

    #[test]
    fn derives_name_from_path_without_spec_file() {
        let dir = TempDir::new().unwrap();

        let spec = resolve_spec(dir.path()).unwrap();

        assert_eq!(spec.name, derive_name_from_path(dir.path()).unwrap());
    }

    #[test]
    fn invalid_spec_file_returns_error() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(SPEC_FILE_NAME), "image: ubuntu:24.04\n").unwrap();

        assert!(resolve_spec(dir.path()).is_err());
    }

    #[test]
    fn explicit_name_skips_spec_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(SPEC_FILE_NAME), "image: ubuntu:24.04\n").unwrap();

        let name = sandbox_name(Some("other".to_string()), dir.path()).unwrap();

        assert_eq!(name, "other");
    }

    #[test]
    fn without_name_uses_working_directory() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(SPEC_FILE_NAME), "name: dev\n").unwrap();

        let name = sandbox_name(None, dir.path()).unwrap();

        assert_eq!(name, "dev");
    }

    #[test]
    fn not_found_status_names_missing_sandbox() {
        let error = describe_status("dev", tonic::Status::not_found("couldn't find it"));

        assert_eq!(error.to_string(), "sandbox dev doesn't exist");
    }

    #[test]
    fn other_status_is_kept() {
        let error = describe_status("dev", tonic::Status::internal("failed to stop sandbox"));

        assert!(
            error.to_string().contains("failed to stop sandbox"),
            "{error}"
        );
    }

    #[test]
    fn encodes_full_path() {
        let name = derive_name_from_path(Path::new("/home/user/my-project.v2")).unwrap();

        assert_eq!(name, "home_user_my_project_v2");
    }

    #[test]
    fn collapses_and_trims_separators() {
        let name = derive_name_from_path(Path::new("//home//user/--project--/")).unwrap();

        assert_eq!(name, "home_user_project");
    }

    #[test]
    fn root_path_returns_error() {
        assert!(derive_name_from_path(Path::new("/")).is_err());
    }

    #[test]
    fn start_request_carries_workspace() {
        let spec = anvil_spec::default_spec("dev".to_string());

        let request = build_start_request(spec, Path::new("/home/user/project"));

        assert_eq!(request.workspace, "/home/user/project");
    }

    #[test]
    fn start_request_carries_image_and_resources_from_spec() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(SPEC_FILE_NAME),
            "name: dev\nimage: alpine:3.22\ninit: false\nresources:\n  cpu: 4\n  memory: 8Gi\n",
        )
        .unwrap();

        let request = build_start_request(resolve_spec(dir.path()).unwrap(), dir.path());
        let resources = request.resources.unwrap();

        assert_eq!(request.image, "alpine:3.22");
        assert_eq!(request.init, Some(false));
        assert_eq!((resources.cpu, resources.memory.as_str()), (4, "8Gi"));
    }

    #[test]
    fn start_request_fills_in_defaults() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(SPEC_FILE_NAME), "name: dev\n").unwrap();

        let request = build_start_request(resolve_spec(dir.path()).unwrap(), dir.path());
        let resources = request.resources.unwrap();
        let defaults = anvil_spec::SandboxResourcesSpec::default();

        assert_eq!(request.image, anvil_spec::DEFAULT_IMAGE);
        assert_eq!(request.init, Some(true));
        assert_eq!(resources.cpu, u32::from(defaults.cpu));
        assert_eq!(resources.memory, defaults.memory);
    }
}

use anvil_spec::SandboxSpec;
use anyhow::{Result, bail};
use clap::ValueEnum;
use serde::Serialize;
use std::path::Path;
use std::time::Duration;
use tokio::time::{Instant, sleep};
use tonic::Code;
use tonic::transport::Channel;

use crate::api::{
    GetSandboxRequest, ListSandboxesRequest, ListSandboxesResponse, RemoveSandboxRequest,
    SandboxResources, SandboxStatus, SandboxSummary, StartSandboxRequest, StopSandboxRequest,
    sandbox_management_service_client::SandboxManagementServiceClient,
};
use crate::table;

pub(crate) const SPEC_FILE_NAME: &str = ".anvil.yml";

/// Starts the sandbox for the working directory, creating it when needed.
pub async fn start_sandbox(
    working_dir: &Path,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<()> {
    let spec = resolve_spec(working_dir)?;
    let name = spec.name.clone();

    client
        .start_sandbox(build_start_request(spec, working_dir))
        .await?;

    let hostname = client
        .get_sandbox(GetSandboxRequest { name })
        .await?
        .into_inner()
        .hostname;

    if !hostname.is_empty() {
        println!("Connect with: ssh {hostname}");
    }

    Ok(())
}

const STARTING_TIMEOUT: Duration = Duration::from_secs(120);
const STARTING_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Makes sure the sandbox exists and is running, creating or starting it when needed.
pub(crate) async fn ensure_running(
    spec: SandboxSpec,
    workspace: &Path,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<()> {
    let name = spec.name.clone();
    let deadline = Instant::now() + STARTING_TIMEOUT;

    loop {
        let response = match client
            .get_sandbox(GetSandboxRequest {
                name: name.to_string(),
            })
            .await
        {
            Ok(response) => response.into_inner(),
            Err(status) if status.code() == Code::NotFound => {
                eprintln!("Creating sandbox {name}...");

                client
                    .start_sandbox(build_start_request(spec, workspace))
                    .await?;

                return Ok(());
            }
            Err(status) => return Err(status.into()),
        };

        match response.status() {
            SandboxStatus::Running => return Ok(()),
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

                return Ok(());
            }
            SandboxStatus::Starting => {
                if Instant::now() >= deadline {
                    bail!("timed out waiting for sandbox {name} to start");
                }

                sleep(STARTING_POLL_INTERVAL).await;
            }
            status @ (SandboxStatus::Stopping | SandboxStatus::Paused) => {
                bail!(
                    "sandbox {name} is {}; try again once it has stopped",
                    format_status(status).to_lowercase()
                );
            }
        }
    }
}

/// Builds a start request from the spec, filling in the default image and resources.
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
    }
}

/// Stops the sandbox for the working directory.
pub async fn stop_sandbox(
    working_dir: &Path,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<()> {
    let name = resolve_spec(working_dir)?.name;

    client.stop_sandbox(StopSandboxRequest { name }).await?;

    Ok(())
}

/// Removes the sandbox for the working directory.
pub async fn remove_sandbox(
    working_dir: &Path,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<()> {
    let name = resolve_spec(working_dir)?.name;

    client.remove_sandbox(RemoveSandboxRequest { name }).await?;

    Ok(())
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
            "name: dev\nimage: alpine:3.22\nresources:\n  cpu: 4\n  memory: 8Gi\n",
        )
        .unwrap();

        let request = build_start_request(resolve_spec(dir.path()).unwrap(), dir.path());
        let resources = request.resources.unwrap();

        assert_eq!(request.image, "alpine:3.22");
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
        assert_eq!(resources.cpu, u32::from(defaults.cpu));
        assert_eq!(resources.memory, defaults.memory);
    }
}

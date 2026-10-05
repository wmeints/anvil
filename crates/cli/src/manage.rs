use anvil_spec::{SandboxResourcesSpec, SandboxSpec};
use anyhow::{Result, bail};
use std::path::Path;
use std::time::Duration;
use tokio::time::{Instant, sleep};
use tonic::Code;
use tonic::transport::Channel;

use crate::api::{
    GetSandboxRequest, ListSandboxesRequest, ListSandboxesResponse, RemoveSandboxRequest,
    SandboxResources, SandboxStatus, StartSandboxRequest, StopSandboxRequest,
    sandbox_management_service_client::SandboxManagementServiceClient,
};

const SPEC_FILE_NAME: &str = ".anvil.yml";

/// Starts the sandbox for the working directory, creating it when needed.
pub async fn start_sandbox(
    working_dir: &Path,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<()> {
    let spec = resolve_spec(working_dir)?;

    client
        .start_sandbox(build_start_request(spec, working_dir))
        .await?;

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

                client
                    .start_sandbox(StartSandboxRequest {
                        name: name.to_string(),
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

/// Builds a start request from the spec, filling in a default image and resources.
/// The workspace is mounted into the sandbox when it's created.
fn build_start_request(mut spec: SandboxSpec, workspace: &Path) -> StartSandboxRequest {
    if spec.image.is_none() {
        spec.image = Some("ubuntu:26.04".to_string());
    }

    if spec.resources.is_none() {
        spec.resources = Some(SandboxResourcesSpec {
            cpu: 2,
            memory: "2 GiB".to_string(),
        })
    }

    StartSandboxRequest {
        name: spec.name,
        image: spec.image.expect("image is required"),
        resources: spec.resources.map(|res| SandboxResources {
            cpu: res.cpu.into(),
            memory: res.memory,
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

/// Prints all sandboxes with their status.
pub async fn list_sandboxes(client: &mut SandboxManagementServiceClient<Channel>) -> Result<()> {
    let response = client.list_sandboxes(ListSandboxesRequest {}).await?;
    let response_data: ListSandboxesResponse = response.into_inner();

    for item in response_data.sandboxes {
        println!("{}\t{}", item.name, format_status(item.status()));
    }

    Ok(())
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

    return status_text.to_string();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

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
}

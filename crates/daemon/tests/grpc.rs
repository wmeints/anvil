//! Integration tests for the gRPC handlers.
//!
//! These tests talk to a real daemon over a unix socket and use the local microsandbox
//! runtime, so they boot real VMs and pull the sandbox image. They only build with the
//! `vm-tests` feature; run them with `cargo test -p anvil-daemon --features vm-tests`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anvil_daemon::api::sandbox_management_service_client::SandboxManagementServiceClient;
use anvil_daemon::api::{
    AttachInput, AttachRequest, AttachResize, AttachResponse, AttachStart, GetSandboxRequest,
    GetSandboxResponse, ListSandboxesRequest, RemoveSandboxRequest, SandboxResources,
    SandboxStatus, StartSandboxRequest, StopSandboxRequest, attach_request, attach_response,
};
use anvil_daemon::secrets::{self, Secret, SecretStore};
use anvil_daemon::server;
use hyper_util::rt::TokioIo;
use microsandbox::Sandbox;
use microsandbox::sandbox::{RootfsSource, SandboxSpec};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep};
use tonic::codegen::tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Code, Streaming};
use tower::service_fn;

const STATUS_TIMEOUT: Duration = Duration::from_secs(120);
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const DEFAULT_SIZE: AttachResize = AttachResize {
    width: 80,
    height: 24,
};

struct TestDaemon {
    socket_path: PathBuf,
    shutdown: Option<oneshot::Sender<()>>,
    handle: JoinHandle<Result<(), server::ServerError>>,
}

impl TestDaemon {
    async fn start(name: &str) -> Self {
        let secrets_path = std::env::temp_dir().join(format!(
            "anvil-it-{}-{}-secrets.yml",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_file(&secrets_path);

        Self::start_with_secrets(name, SecretStore::new(secrets_path)).await
    }

    async fn start_with_secrets(name: &str, secrets: SecretStore) -> Self {
        let socket_path =
            std::env::temp_dir().join(format!("anvil-it-{}-{}.sock", std::process::id(), name));
        let _ = std::fs::remove_file(&socket_path);

        let (tx, rx) = oneshot::channel();
        let handle = tokio::spawn(serve_until(socket_path.clone(), secrets, rx));

        wait_for_socket(&socket_path).await;

        Self {
            socket_path,
            shutdown: Some(tx),
            handle,
        }
    }

    async fn client(&self) -> SandboxManagementServiceClient<Channel> {
        let path = self.socket_path.clone();

        // The HTTP endpoint isn't used; it only shows up in the authority header.
        let channel = Endpoint::try_from("http://localhost")
            .unwrap()
            .connect_with_connector(service_fn(move |_: Uri| connect_unix(path.clone())))
            .await
            .expect("failed to connect to test daemon");

        SandboxManagementServiceClient::new(channel)
    }

    async fn stop(mut self) {
        self.shutdown.take().unwrap().send(()).unwrap();
        self.handle
            .await
            .unwrap()
            .expect("daemon exited with an error");
    }
}

/// Serves the daemon on the socket until `stop` fires.
async fn serve_until(
    path: PathBuf,
    secrets: SecretStore,
    stop: oneshot::Receiver<()>,
) -> Result<(), server::ServerError> {
    server::serve(&path, secrets, async {
        let _ = stop.await;
    })
    .await
}

/// Connects to the daemon socket for a gRPC channel.
async fn connect_unix(path: PathBuf) -> std::io::Result<TokioIo<UnixStream>> {
    Ok(TokioIo::new(UnixStream::connect(path).await?))
}

async fn wait_for_socket(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);

    while UnixStream::connect(path).await.is_err() {
        assert!(Instant::now() < deadline, "daemon did not start listening");
        sleep(Duration::from_millis(10)).await;
    }
}

/// Stops and removes a sandbox and its workspace, including ones left over from an earlier
/// (failed) run.
async fn remove_sandbox(name: &str) {
    if let Ok(sb) = Sandbox::get(name).await {
        let _ = sb.stop().await;
        let _ = sb.remove().await;
    }

    let _ = std::fs::remove_dir_all(workspace_path(name));
}

async fn sandbox_status(
    client: &mut SandboxManagementServiceClient<Channel>,
    name: &str,
) -> Option<SandboxStatus> {
    client
        .list_sandboxes(ListSandboxesRequest {})
        .await
        .expect("list_sandboxes failed")
        .into_inner()
        .sandboxes
        .into_iter()
        .find(|sb| sb.name == name)
        .map(|sb| SandboxStatus::try_from(sb.status).expect("invalid sandbox status"))
}

async fn wait_for_status(
    client: &mut SandboxManagementServiceClient<Channel>,
    name: &str,
    expected: SandboxStatus,
) {
    let deadline = Instant::now() + STATUS_TIMEOUT;

    loop {
        let status = sandbox_status(client, name).await;

        if status == Some(expected) {
            return;
        }

        assert!(
            Instant::now() < deadline,
            "sandbox {name} did not reach {expected:?}, last status: {status:?}"
        );

        sleep(POLL_INTERVAL).await;
    }
}

/// Returns the host directory used as the workspace of a test sandbox. It's named after the
/// sandbox only, so `remove_sandbox` also cleans up workspaces left over from a failed run.
fn workspace_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{name}-ws"))
}

/// Creates an empty host directory to mount as the workspace of a test sandbox.
fn test_workspace(name: &str) -> PathBuf {
    let workspace = workspace_path(name);
    let _ = std::fs::remove_dir_all(&workspace);
    std::fs::create_dir_all(&workspace).expect("failed to create test workspace");

    workspace
}

/// Returns the path the daemon mounts a workspace at in the guest.
fn guest_workspace(workspace: &Path) -> String {
    let leaf = workspace.file_name().unwrap().to_string_lossy();

    format!("/workspaces/{leaf}")
}

/// Image the tests boot, independent of the default image.
const TEST_IMAGE: &str = "ubuntu:26.04";

fn start_request(name: &str) -> StartSandboxRequest {
    StartSandboxRequest {
        name: name.to_string(),
        workspace: test_workspace(name).to_string_lossy().into_owned(),
        // The default image is only published on release, so it doesn't exist for unreleased versions.
        image: TEST_IMAGE.to_string(),
        ..Default::default()
    }
}

/// Returns the sandbox as the daemon reports it.
async fn get_sandbox(
    client: &mut SandboxManagementServiceClient<Channel>,
    name: &str,
) -> GetSandboxResponse {
    client
        .get_sandbox(GetSandboxRequest {
            name: name.to_string(),
        })
        .await
        .expect("failed to get sandbox")
        .into_inner()
}

/// Stops the sandbox and waits until it has stopped.
async fn stop_sandbox(client: &mut SandboxManagementServiceClient<Channel>, name: &str) {
    client
        .stop_sandbox(StopSandboxRequest {
            name: name.to_string(),
        })
        .await
        .expect("failed to stop sandbox");
    wait_for_status(client, name, SandboxStatus::Stopped).await;
}

/// Returns the configuration microsandbox stores for the sandbox.
async fn sandbox_spec(name: &str) -> SandboxSpec {
    Sandbox::get(name)
        .await
        .expect("failed to get sandbox")
        .config()
        .expect("failed to read sandbox config")
        .spec
}

#[tokio::test]
async fn list_sandboxes_succeeds() {
    let daemon = TestDaemon::start("list").await;
    let mut client = daemon.client().await;

    client
        .list_sandboxes(ListSandboxesRequest {})
        .await
        .expect("list_sandboxes failed");

    daemon.stop().await;
}

#[tokio::test]
async fn stop_sandbox_returns_not_found_for_unknown_sandbox() {
    let daemon = TestDaemon::start("stop-unknown").await;
    let mut client = daemon.client().await;

    let status = client
        .stop_sandbox(StopSandboxRequest {
            name: "anvil-it-does-not-exist".to_string(),
        })
        .await
        .expect_err("stopping an unknown sandbox should fail");

    assert_eq!(status.code(), Code::NotFound);

    daemon.stop().await;
}

#[tokio::test]
async fn get_sandbox_returns_not_found_for_unknown_sandbox() {
    let daemon = TestDaemon::start("get-unknown").await;
    let mut client = daemon.client().await;

    let status = client
        .get_sandbox(GetSandboxRequest {
            name: "anvil-it-does-not-exist".to_string(),
        })
        .await
        .expect_err("getting an unknown sandbox should fail");

    assert_eq!(status.code(), Code::NotFound);

    daemon.stop().await;
}

#[tokio::test]
async fn get_sandbox_returns_name_and_status() {
    const NAME: &str = "anvil-it-get";
    remove_sandbox(NAME).await;

    let daemon = TestDaemon::start("get").await;
    let mut client = daemon.client().await;

    start_running_sandbox(&mut client, NAME).await;

    let sandbox = get_sandbox(&mut client, NAME).await;

    assert_eq!(sandbox.name, NAME);
    assert_eq!(sandbox.status(), SandboxStatus::Running);

    stop_sandbox(&mut client, NAME).await;

    assert_eq!(
        get_sandbox(&mut client, NAME).await.status(),
        SandboxStatus::Stopped
    );

    daemon.stop().await;
    remove_sandbox(NAME).await;
}

#[tokio::test]
async fn sandbox_lifecycle() {
    const NAME: &str = "anvil-it-lifecycle";
    remove_sandbox(NAME).await;

    let daemon = TestDaemon::start("lifecycle").await;
    let mut client = daemon.client().await;

    // Starting an unknown sandbox creates it.
    client
        .start_sandbox(start_request(NAME))
        .await
        .expect("failed to create sandbox");
    wait_for_status(&mut client, NAME, SandboxStatus::Running).await;

    client
        .stop_sandbox(StopSandboxRequest {
            name: NAME.to_string(),
        })
        .await
        .expect("failed to stop sandbox");
    wait_for_status(&mut client, NAME, SandboxStatus::Stopped).await;

    // Starting an existing sandbox restarts it.
    client
        .start_sandbox(start_request(NAME))
        .await
        .expect("failed to restart sandbox");
    wait_for_status(&mut client, NAME, SandboxStatus::Running).await;

    daemon.stop().await;
    remove_sandbox(NAME).await;
}

#[tokio::test]
async fn start_sandbox_uses_requested_image_and_resources() {
    const NAME: &str = "anvil-it-resources";
    remove_sandbox(NAME).await;

    let daemon = TestDaemon::start("resources").await;
    let mut client = daemon.client().await;

    client
        .start_sandbox(StartSandboxRequest {
            image: "alpine:3.22".to_string(),
            resources: Some(SandboxResources {
                cpu: 1,
                memory: "1 GiB".to_string(),
            }),
            ..start_request(NAME)
        })
        .await
        .expect("failed to create sandbox");
    wait_for_status(&mut client, NAME, SandboxStatus::Running).await;

    let spec = sandbox_spec(NAME).await;

    assert!(
        matches!(&spec.image, RootfsSource::Oci(oci) if oci.reference.contains("alpine")),
        "unexpected image: {:?}",
        spec.image
    );
    assert_eq!(spec.resources.cpus, 1);
    assert_eq!(spec.resources.memory_mib, 1024);

    daemon.stop().await;
    remove_sandbox(NAME).await;
}

#[tokio::test]
async fn start_sandbox_rejects_invalid_resources() {
    const NAME: &str = "anvil-it-bad-resources";
    remove_sandbox(NAME).await;

    let daemon = TestDaemon::start("bad-resources").await;
    let mut client = daemon.client().await;

    let status = client
        .start_sandbox(StartSandboxRequest {
            resources: Some(SandboxResources {
                cpu: 2,
                memory: "lots".to_string(),
            }),
            ..start_request(NAME)
        })
        .await
        .expect_err("start_sandbox should reject invalid memory");

    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(Sandbox::get(NAME).await.is_err(), "sandbox was created");

    daemon.stop().await;
    remove_sandbox(NAME).await;
}

#[tokio::test]
async fn remove_sandbox_returns_not_found_for_unknown_sandbox() {
    let daemon = TestDaemon::start("remove-unknown").await;
    let mut client = daemon.client().await;

    let status = client
        .remove_sandbox(RemoveSandboxRequest {
            name: "anvil-it-does-not-exist".to_string(),
        })
        .await
        .expect_err("removing an unknown sandbox should fail");

    assert_eq!(status.code(), Code::NotFound);

    daemon.stop().await;
}

#[tokio::test]
async fn remove_sandbox_removes_stopped_sandbox() {
    const NAME: &str = "anvil-it-remove";
    remove_sandbox(NAME).await;

    let daemon = TestDaemon::start("remove").await;
    let mut client = daemon.client().await;

    client
        .start_sandbox(start_request(NAME))
        .await
        .expect("failed to create sandbox");
    wait_for_status(&mut client, NAME, SandboxStatus::Running).await;

    client
        .stop_sandbox(StopSandboxRequest {
            name: NAME.to_string(),
        })
        .await
        .expect("failed to stop sandbox");
    wait_for_status(&mut client, NAME, SandboxStatus::Stopped).await;

    client
        .remove_sandbox(RemoveSandboxRequest {
            name: NAME.to_string(),
        })
        .await
        .expect("failed to remove sandbox");

    assert_eq!(sandbox_status(&mut client, NAME).await, None);

    daemon.stop().await;
    remove_sandbox(NAME).await;
}

fn attach_start(name: &str, command: &str, args: &[&str], size: AttachResize) -> AttachRequest {
    AttachRequest {
        message: Some(attach_request::Message::Start(AttachStart {
            name: name.to_string(),
            command: command.to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            size: Some(size),
        })),
    }
}

fn attach_input(data: &[u8]) -> AttachRequest {
    AttachRequest {
        message: Some(attach_request::Message::Input(AttachInput {
            data: data.to_vec(),
        })),
    }
}

fn attach_resize(width: u32, height: u32) -> AttachRequest {
    AttachRequest {
        message: Some(attach_request::Message::Resize(AttachResize {
            width,
            height,
        })),
    }
}

/// Collects session output until the session reports its exit code.
async fn collect_session(mut responses: Streaming<AttachResponse>) -> (String, i32) {
    let mut output = Vec::new();

    loop {
        let response = tokio::time::timeout(STATUS_TIMEOUT, responses.message())
            .await
            .expect("session did not finish in time")
            .expect("attach stream failed")
            .expect("attach stream closed before the session exited");

        match response.message {
            Some(attach_response::Message::Output(data)) => output.extend(data),
            Some(attach_response::Message::ExitCode(code)) => {
                return (String::from_utf8_lossy(&output).into_owned(), code);
            }
            None => {}
        }
    }
}

/// Sends the start message and returns the sender for further messages and the session output.
async fn open_session(
    client: &mut SandboxManagementServiceClient<Channel>,
    start: AttachRequest,
) -> (mpsc::Sender<AttachRequest>, Streaming<AttachResponse>) {
    let (tx, rx) = mpsc::channel(4);
    tx.send(start).await.unwrap();

    let responses = client
        .attach(ReceiverStream::new(rx))
        .await
        .expect("attach failed")
        .into_inner();

    (tx, responses)
}

/// Runs a command in the sandbox without input and returns its output and exit code.
async fn run_command(
    client: &mut SandboxManagementServiceClient<Channel>,
    sandbox: &str,
    command: &str,
    args: &[&str],
) -> (String, i32) {
    let (tx, responses) =
        open_session(client, attach_start(sandbox, command, args, DEFAULT_SIZE)).await;
    let result = collect_session(responses).await;

    // Closing the input ends the session, so keep it open until the session has exited.
    drop(tx);

    result
}

async fn start_running_sandbox(client: &mut SandboxManagementServiceClient<Channel>, name: &str) {
    start_and_wait(client, start_request(name)).await;
}

/// Starts a sandbox with the request and waits until it runs.
async fn start_and_wait(
    client: &mut SandboxManagementServiceClient<Channel>,
    request: StartSandboxRequest,
) {
    let name = request.name.clone();

    client
        .start_sandbox(request)
        .await
        .expect("failed to create sandbox");
    wait_for_status(client, &name, SandboxStatus::Running).await;
}

#[tokio::test]
async fn attach_rejects_missing_start() {
    let daemon = TestDaemon::start("attach-no-start").await;
    let mut client = daemon.client().await;

    let (tx, rx) = mpsc::channel(4);
    tx.send(attach_input(b"hi\n")).await.unwrap();

    let status = client
        .attach(ReceiverStream::new(rx))
        .await
        .expect_err("attach without start should fail");

    assert_eq!(status.code(), Code::InvalidArgument);

    daemon.stop().await;
}

#[tokio::test]
async fn attach_returns_not_found_for_unknown_sandbox() {
    let daemon = TestDaemon::start("attach-unknown").await;
    let mut client = daemon.client().await;

    let (tx, rx) = mpsc::channel(4);
    tx.send(attach_start(
        "anvil-it-does-not-exist",
        "sh",
        &[],
        DEFAULT_SIZE,
    ))
    .await
    .unwrap();

    let status = client
        .attach(ReceiverStream::new(rx))
        .await
        .expect_err("attaching to an unknown sandbox should fail");

    assert_eq!(status.code(), Code::NotFound);

    daemon.stop().await;
}

#[tokio::test]
async fn attach_runs_command_and_streams_output() {
    const NAME: &str = "anvil-it-attach";
    remove_sandbox(NAME).await;

    let daemon = TestDaemon::start("attach").await;
    let mut client = daemon.client().await;
    start_running_sandbox(&mut client, NAME).await;

    let start = attach_start(NAME, "sh", &["-c", "read x; echo got:$x"], DEFAULT_SIZE);
    let (tx, responses) = open_session(&mut client, start).await;
    tx.send(attach_input(b"hi\n")).await.unwrap();

    let (output, code) = collect_session(responses).await;
    drop(tx);

    assert!(output.contains("got:hi"), "unexpected output: {output:?}");
    assert_eq!(code, 0);

    daemon.stop().await;
    remove_sandbox(NAME).await;
}

#[tokio::test]
async fn attach_applies_window_size_and_resize() {
    const NAME: &str = "anvil-it-attach-resize";
    remove_sandbox(NAME).await;

    let daemon = TestDaemon::start("attach-resize").await;
    let mut client = daemon.client().await;
    start_running_sandbox(&mut client, NAME).await;

    let start = attach_start(
        NAME,
        "sh",
        &["-c", "stty size; read x; stty size"],
        AttachResize {
            width: 100,
            height: 40,
        },
    );
    let (tx, responses) = open_session(&mut client, start).await;

    // Resize before releasing the `read` so the second `stty size` sees the new size.
    sleep(Duration::from_millis(500)).await;
    tx.send(attach_resize(120, 50)).await.unwrap();
    sleep(Duration::from_millis(200)).await;
    tx.send(attach_input(b"\n")).await.unwrap();

    let (output, code) = collect_session(responses).await;
    drop(tx);

    assert!(
        output.contains("40 100"),
        "initial size not applied: {output:?}"
    );
    assert!(output.contains("50 120"), "resize not applied: {output:?}");
    assert_eq!(code, 0);

    daemon.stop().await;
    remove_sandbox(NAME).await;
}

/// Waits until no process with the name runs in the sandbox.
async fn wait_for_process_exit(sandbox: &str, process: &str) {
    let sb = Sandbox::get(sandbox)
        .await
        .expect("sandbox disappeared")
        .connect()
        .await
        .expect("failed to connect to sandbox");
    let deadline = Instant::now() + Duration::from_secs(10);
    let check = format!("pgrep -x {process} || true");

    while !process_output(&sb, &check).await.is_empty() {
        assert!(Instant::now() < deadline, "{process} is still running");
        sleep(POLL_INTERVAL).await;
    }
}

/// Runs a shell command in the sandbox and returns its standard output.
async fn process_output(sb: &Sandbox, command: &str) -> Vec<u8> {
    sb.exec("sh", ["-c", command])
        .await
        .expect("failed to check for session process")
        .stdout_bytes()
        .to_vec()
}

#[tokio::test]
async fn attach_disconnect_ends_session() {
    const NAME: &str = "anvil-it-attach-disconnect";
    remove_sandbox(NAME).await;

    let daemon = TestDaemon::start("attach-disconnect").await;
    let mut client = daemon.client().await;
    start_running_sandbox(&mut client, NAME).await;

    let start = attach_start(NAME, "sleep", &["300"], DEFAULT_SIZE);
    let (tx, responses) = open_session(&mut client, start).await;

    sleep(Duration::from_millis(500)).await;
    drop(tx);
    drop(responses);

    wait_for_process_exit(NAME, "sleep").await;

    daemon.stop().await;
    remove_sandbox(NAME).await;
}

#[tokio::test]
async fn start_sandbox_rejects_relative_workspace() {
    const NAME: &str = "anvil-it-relative-workspace";
    remove_sandbox(NAME).await;

    let daemon = TestDaemon::start("relative-workspace").await;
    let mut client = daemon.client().await;

    let status = client
        .start_sandbox(StartSandboxRequest {
            name: NAME.to_string(),
            workspace: "project".to_string(),
            ..Default::default()
        })
        .await
        .expect_err("a relative workspace should be rejected");

    assert_eq!(status.code(), Code::InvalidArgument);

    daemon.stop().await;
}

#[tokio::test]
async fn workspace_is_mounted_read_write() {
    const NAME: &str = "anvil-it-workspace";
    remove_sandbox(NAME).await;

    let request = start_request(NAME);
    let workspace = PathBuf::from(&request.workspace);
    let guest_path = guest_workspace(&workspace);
    std::fs::write(workspace.join("from-host.txt"), "hello from host").unwrap();

    let daemon = TestDaemon::start("workspace").await;
    let mut client = daemon.client().await;
    start_and_wait(&mut client, request).await;

    let script = format!(
        "pwd; cat {guest_path}/from-host.txt; echo; echo hello from guest > {guest_path}/from-guest.txt"
    );
    let (output, code) = run_command(&mut client, NAME, "sh", &["-c", &script]).await;

    assert_eq!(code, 0, "unexpected output: {output:?}");
    assert!(
        output.contains(&guest_path),
        "session didn't start in the workspace: {output:?}"
    );
    assert!(
        output.contains("hello from host"),
        "unexpected output: {output:?}"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("from-guest.txt")).unwrap(),
        "hello from guest\n"
    );

    daemon.stop().await;
    remove_sandbox(NAME).await;
}

fn test_secret(name: &str) -> Secret {
    Secret::new(
        name.to_string(),
        "anvil-it-secret-value".to_string(),
        vec!["example.com".to_string()],
    )
    .unwrap()
}

/// Prints the environment variable in the sandbox and returns the output.
async fn print_env(
    client: &mut SandboxManagementServiceClient<Channel>,
    sandbox: &str,
    var: &str,
) -> String {
    let (output, code) = run_print_env(client, sandbox, var).await;
    assert_eq!(code, 0, "printenv {var} failed: {output:?}");

    output
}

/// Runs `printenv` for the variable in the sandbox and returns its output and exit code.
async fn run_print_env(
    client: &mut SandboxManagementServiceClient<Channel>,
    sandbox: &str,
    var: &str,
) -> (String, i32) {
    run_command(client, sandbox, "printenv", &[var]).await
}

/// Stops the sandbox and starts it again.
async fn restart_sandbox(client: &mut SandboxManagementServiceClient<Channel>, name: &str) {
    stop_sandbox(client, name).await;
    start_running_sandbox(client, name).await;
}

#[tokio::test]
async fn new_sandbox_sees_secret_placeholder_only() {
    const NAME: &str = "anvil-it-secret-new";
    remove_sandbox(NAME).await;

    let dir = tempfile::tempdir().unwrap();
    let store = SecretStore::new(dir.path().join("secrets.yml"));
    store.set(test_secret("ANVIL_IT_TOKEN")).unwrap();

    let daemon = TestDaemon::start_with_secrets("secret-new", store).await;
    let mut client = daemon.client().await;
    start_running_sandbox(&mut client, NAME).await;

    let output = print_env(&mut client, NAME, "ANVIL_IT_TOKEN").await;

    assert!(
        output.contains("$MSB_ANVIL_IT_TOKEN"),
        "unexpected output: {output:?}"
    );
    assert!(!output.contains("anvil-it-secret-value"));

    daemon.stop().await;
    remove_sandbox(NAME).await;
}

#[tokio::test]
async fn existing_sandbox_sees_added_secret_after_restart() {
    const NAME: &str = "anvil-it-secret-existing";
    remove_sandbox(NAME).await;

    let daemon = TestDaemon::start("secret-existing").await;
    let mut client = daemon.client().await;
    start_running_sandbox(&mut client, NAME).await;

    let handle = Sandbox::get(NAME).await.unwrap();
    secrets::add_to_sandbox(&handle, &test_secret("ANVIL_IT_TOKEN"))
        .await
        .expect("failed to add secret");

    restart_sandbox(&mut client, NAME).await;

    let output = print_env(&mut client, NAME, "ANVIL_IT_TOKEN").await;

    assert!(
        output.contains("$MSB_ANVIL_IT_TOKEN"),
        "unexpected output: {output:?}"
    );
    assert!(!output.contains("anvil-it-secret-value"));

    daemon.stop().await;
    remove_sandbox(NAME).await;
}

#[tokio::test]
async fn existing_sandbox_loses_removed_secret_after_restart() {
    const NAME: &str = "anvil-it-secret-removed";
    remove_sandbox(NAME).await;

    let dir = tempfile::tempdir().unwrap();
    let store = SecretStore::new(dir.path().join("secrets.yml"));
    store.set(test_secret("ANVIL_IT_TOKEN")).unwrap();

    let daemon = TestDaemon::start_with_secrets("secret-removed", store).await;
    let mut client = daemon.client().await;
    start_running_sandbox(&mut client, NAME).await;

    // The RemoveSecret RPC would change every anvil sandbox on the host, so remove the secret
    // from the test sandbox only.
    let handle = Sandbox::get(NAME).await.unwrap();
    secrets::remove_from_sandbox(&handle, "ANVIL_IT_TOKEN")
        .await
        .expect("failed to remove secret");
    restart_sandbox(&mut client, NAME).await;

    let (output, code) = run_print_env(&mut client, NAME, "ANVIL_IT_TOKEN").await;

    assert_ne!(code, 0, "secret is still set: {output:?}");

    daemon.stop().await;
    remove_sandbox(NAME).await;
}

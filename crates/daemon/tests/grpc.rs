//! Integration tests for the gRPC handlers.
//!
//! These tests talk to a real daemon over a unix socket and use the local microsandbox
//! runtime, so they boot real VMs and pull the sandbox image. They only build with the
//! `vm-tests` feature; run them with `cargo test -p anvil-daemon --features vm-tests`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anvil_daemon::api::sandbox_management_service_client::SandboxManagementServiceClient;
use anvil_daemon::api::{
    AttachInput, AttachRequest, AttachResize, AttachStart, GetSandboxRequest, ListSandboxesRequest,
    RemoveSandboxRequest, SandboxStatus, StartSandboxRequest, StopSandboxRequest, attach_request,
    attach_response,
};
use anvil_daemon::server;
use hyper_util::rt::TokioIo;
use microsandbox::Sandbox;
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep};
use tonic::Code;
use tonic::codegen::tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

const STATUS_TIMEOUT: Duration = Duration::from_secs(120);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

struct TestDaemon {
    socket_path: PathBuf,
    shutdown: Option<oneshot::Sender<()>>,
    handle: JoinHandle<Result<(), server::ServerError>>,
}

impl TestDaemon {
    async fn start(name: &str) -> Self {
        let socket_path =
            std::env::temp_dir().join(format!("anvil-it-{}-{}.sock", std::process::id(), name));
        let _ = std::fs::remove_file(&socket_path);

        let (tx, rx) = oneshot::channel();
        let path = socket_path.clone();
        let handle = tokio::spawn(async move {
            server::serve(&path, async {
                let _ = rx.await;
            })
            .await
        });

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
        let channel =
            Endpoint::try_from("http://localhost")
                .unwrap()
                .connect_with_connector(service_fn(move |_: Uri| {
                    let path = path.clone();
                    async move {
                        Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?))
                    }
                }))
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

fn start_request(name: &str) -> StartSandboxRequest {
    StartSandboxRequest {
        name: name.to_string(),
        workspace: test_workspace(name).to_string_lossy().into_owned(),
        ..Default::default()
    }
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

    client
        .start_sandbox(start_request(NAME))
        .await
        .expect("failed to create sandbox");
    wait_for_status(&mut client, NAME, SandboxStatus::Running).await;

    let sandbox = client
        .get_sandbox(GetSandboxRequest {
            name: NAME.to_string(),
        })
        .await
        .expect("failed to get sandbox")
        .into_inner();

    assert_eq!(sandbox.name, NAME);
    assert_eq!(sandbox.status(), SandboxStatus::Running);

    client
        .stop_sandbox(StopSandboxRequest {
            name: NAME.to_string(),
        })
        .await
        .expect("failed to stop sandbox");
    wait_for_status(&mut client, NAME, SandboxStatus::Stopped).await;

    let sandbox = client
        .get_sandbox(GetSandboxRequest {
            name: NAME.to_string(),
        })
        .await
        .expect("failed to get sandbox")
        .into_inner();

    assert_eq!(sandbox.status(), SandboxStatus::Stopped);

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

fn attach_start(
    name: &str,
    command: &str,
    args: &[&str],
    width: u32,
    height: u32,
) -> AttachRequest {
    AttachRequest {
        message: Some(attach_request::Message::Start(AttachStart {
            name: name.to_string(),
            command: command.to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            size: Some(AttachResize { width, height }),
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
async fn collect_session(
    mut responses: tonic::Streaming<anvil_daemon::api::AttachResponse>,
) -> (String, i32) {
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

async fn start_running_sandbox(client: &mut SandboxManagementServiceClient<Channel>, name: &str) {
    client
        .start_sandbox(start_request(name))
        .await
        .expect("failed to create sandbox");
    wait_for_status(client, name, SandboxStatus::Running).await;
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
    tx.send(attach_start("anvil-it-does-not-exist", "sh", &[], 80, 24))
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

    let (tx, rx) = mpsc::channel(4);
    tx.send(attach_start(
        NAME,
        "sh",
        &["-c", "read x; echo got:$x"],
        80,
        24,
    ))
    .await
    .unwrap();
    tx.send(attach_input(b"hi\n")).await.unwrap();

    let responses = client
        .attach(ReceiverStream::new(rx))
        .await
        .expect("attach failed")
        .into_inner();

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

    let (tx, rx) = mpsc::channel(4);
    tx.send(attach_start(
        NAME,
        "sh",
        &["-c", "stty size; read x; stty size"],
        100,
        40,
    ))
    .await
    .unwrap();

    let responses = client
        .attach(ReceiverStream::new(rx))
        .await
        .expect("attach failed")
        .into_inner();

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

#[tokio::test]
async fn attach_disconnect_ends_session() {
    const NAME: &str = "anvil-it-attach-disconnect";
    remove_sandbox(NAME).await;

    let daemon = TestDaemon::start("attach-disconnect").await;
    let mut client = daemon.client().await;
    start_running_sandbox(&mut client, NAME).await;

    let (tx, rx) = mpsc::channel(4);
    tx.send(attach_start(NAME, "sleep", &["300"], 80, 24))
        .await
        .unwrap();

    let responses = client
        .attach(ReceiverStream::new(rx))
        .await
        .expect("attach failed")
        .into_inner();

    sleep(Duration::from_millis(500)).await;
    drop(tx);
    drop(responses);

    let sb = Sandbox::get(NAME)
        .await
        .expect("sandbox disappeared")
        .connect()
        .await
        .expect("failed to connect to sandbox");
    let deadline = Instant::now() + Duration::from_secs(10);

    loop {
        let output = sb
            .exec("sh", ["-c", "pgrep -x sleep || true"])
            .await
            .expect("failed to check for session process");

        if output.stdout_bytes().is_empty() {
            break;
        }

        assert!(Instant::now() < deadline, "session process still running");
        sleep(POLL_INTERVAL).await;
    }

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

    let workspace = test_workspace(NAME);
    let leaf = workspace
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    std::fs::write(workspace.join("from-host.txt"), "hello from host").unwrap();

    let daemon = TestDaemon::start("workspace").await;
    let mut client = daemon.client().await;
    client
        .start_sandbox(StartSandboxRequest {
            name: NAME.to_string(),
            workspace: workspace.to_string_lossy().into_owned(),
            ..Default::default()
        })
        .await
        .expect("failed to create sandbox");
    wait_for_status(&mut client, NAME, SandboxStatus::Running).await;

    let script = format!(
        "pwd; cat /workspaces/{leaf}/from-host.txt; echo; echo hello from guest > /workspaces/{leaf}/from-guest.txt"
    );
    let (tx, rx) = mpsc::channel(4);
    tx.send(attach_start(NAME, "sh", &["-c", &script], 80, 24))
        .await
        .unwrap();

    let responses = client
        .attach(ReceiverStream::new(rx))
        .await
        .expect("attach failed")
        .into_inner();

    let (output, code) = collect_session(responses).await;
    drop(tx);

    assert_eq!(code, 0, "unexpected output: {output:?}");
    assert!(
        output.contains(&format!("/workspaces/{leaf}")),
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

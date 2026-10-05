use crate::api::sandbox_management_service_server::{
    SandboxManagementService, SandboxManagementServiceServer,
};
use crate::api::{
    AttachRequest, AttachResize, AttachResponse, GetSandboxRequest, GetSandboxResponse,
    ListSandboxesRequest, ListSandboxesResponse, RemoveSandboxRequest, RemoveSandboxResponse,
    SandboxStatus, SandboxSummary, SshTunnelRequest, SshTunnelResponse, StartSandboxRequest,
    StartSandboxResponse, StopSandboxRequest, StopSandboxResponse, attach_request, attach_response,
    ssh_tunnel_request,
};
use crate::ssh;
use anyhow::Result;
use async_trait::async_trait;
use microsandbox::sandbox::exec::{ExecControl, ExecEvent, ExecHandle, ExecSink};
use microsandbox::sandbox::{HostPermissions, SandboxHandle};
use microsandbox::{MicrosandboxError, Sandbox};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::pin::Pin;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tonic::codegen::tokio_stream::Stream;
use tonic::codegen::tokio_stream::wrappers::{ReceiverStream, UnixListenerStream};
use tonic::transport::Server;
use tonic::{Request, Response, Status, Streaming};

/// Errors that can occur while running the gRPC server.
#[derive(Error, Debug)]
pub enum ServerError {
    #[error("socket is already in use")]
    SocketAlreadyInUse(),
    #[error("server can't listen: {0}")]
    FailedToListen(#[from] tonic::transport::Error),
    #[error("invalid socket path")]
    InvalidSocketPath(#[from] std::io::Error),
}

/// gRPC service implementation that manages sandboxes.
#[derive(Default)]
pub struct AnvilServer {}

/// Stream of responses sent back to the client during an attached session.
type AttachStream = Pin<Box<dyn Stream<Item = Result<AttachResponse, Status>> + Send>>;

/// Stream of SSH protocol bytes sent back to the client through a tunnel.
type SshTunnelStream = Pin<Box<dyn Stream<Item = Result<SshTunnelResponse, Status>> + Send>>;

/// Buffer size of the in-memory pipe between a tunnel and its SSH server, per direction.
const TUNNEL_BUFFER_SIZE: usize = 256 * 1024;

/// Largest chunk of SSH protocol bytes sent to the client in a single message.
const TUNNEL_CHUNK_SIZE: usize = 32 * 1024;

#[async_trait]
impl SandboxManagementService for AnvilServer {
    type AttachStream = AttachStream;
    type SshTunnelStream = SshTunnelStream;

    /// Starts an existing sandbox or creates a new one when it doesn't exist.
    async fn start_sandbox(
        &self,
        request: Request<StartSandboxRequest>,
    ) -> Result<Response<StartSandboxResponse>, Status> {
        let request_data = request.into_inner();

        let existing_sb = Sandbox::get(&request_data.name).await;

        if let Ok(existing_sb) = existing_sb {
            // Sandboxes created before SSH support have no host name yet.
            if ssh::hostname_of(&existing_sb).is_none() {
                let workspace = match request_data.workspace.as_str() {
                    "" => request_data.name.as_str(),
                    workspace => workspace,
                };
                let hostname = ssh::pick_hostname(workspace, &taken_hostnames().await?);

                if let Err(err) = existing_sb
                    .modify()
                    .label(ssh::HOSTNAME_LABEL, &hostname)
                    .apply()
                    .await
                {
                    tracing::warn!("failed to assign host name to {}: {err}", request_data.name);
                }
            }

            existing_sb
                .start_detached()
                .await
                .map_err(|_| Status::internal("failed to start sandbox"))?;

            tracing::info!("started sandbox {}", request_data.name);
        } else {
            let guest_path = workspace_mount_path(&request_data.workspace)?;
            let hostname = ssh::pick_hostname(&request_data.workspace, &taken_hostnames().await?);

            Sandbox::builder(&request_data.name)
                .image("ubuntu:26.04")
                .label(ssh::HOSTNAME_LABEL, &hostname)
                // Mounted read/write; Mirror propagates guest chmod changes to the host files.
                .volume(&guest_path, |m| {
                    m.bind(&request_data.workspace)
                        .host_permissions(HostPermissions::Mirror)
                })
                .workdir(&guest_path)
                .detached(true)
                .create()
                .await
                .map_err(|_| Status::internal("failed to create sandbox"))?;

            tracing::info!("created sandbox {} as {hostname}", request_data.name);
        }

        sync_ssh_config().await;

        Ok(Response::new(StartSandboxResponse {}))
    }

    /// Stops a running sandbox.
    async fn stop_sandbox(
        &self,
        request: Request<StopSandboxRequest>,
    ) -> Result<Response<StopSandboxResponse>, Status> {
        let sandbox_name = request.into_inner().name;

        let sb = Sandbox::get(sandbox_name.as_str())
            .await
            .map_err(|err| match err {
                MicrosandboxError::SandboxNotFound(_) => {
                    Status::not_found("couldn't find specified sandbox")
                }
                _ => Status::internal("invalid command"),
            })?;

        sb.stop()
            .await
            .map_err(|_| Status::internal("failed to stop sandbox"))?;

        Ok(Response::new(StopSandboxResponse {}))
    }

    /// Lists all sandboxes with their status.
    async fn list_sandboxes(
        &self,
        _request: Request<ListSandboxesRequest>,
    ) -> Result<Response<ListSandboxesResponse>, Status> {
        let sandboxes = list_all_sandboxes()
            .await?
            .iter()
            .map(|item| SandboxSummary {
                name: item.name().to_string(),
                status: map_sandbox_status(item.status_snapshot()),
                hostname: ssh::hostname_of(item).unwrap_or_default(),
            })
            .collect();

        Ok(Response::new(ListSandboxesResponse { sandboxes }))
    }

    /// Returns the name and status of a single sandbox.
    async fn get_sandbox(
        &self,
        request: Request<GetSandboxRequest>,
    ) -> Result<Response<GetSandboxResponse>, Status> {
        let request_data = request.into_inner();

        let sb = Sandbox::get(request_data.name.as_str())
            .await
            .map_err(|err| match err {
                MicrosandboxError::SandboxNotFound(_) => {
                    Status::not_found("couldn't find specified sandbox")
                }
                _ => Status::internal("failed to get sandbox"),
            })?;

        Ok(Response::new(GetSandboxResponse {
            name: sb.name().to_string(),
            status: map_sandbox_status(sb.status_snapshot()),
            hostname: ssh::hostname_of(&sb).unwrap_or_default(),
        }))
    }

    /// Removes a sandbox.
    async fn remove_sandbox(
        &self,
        request: Request<RemoveSandboxRequest>,
    ) -> std::result::Result<Response<RemoveSandboxResponse>, Status> {
        let request_data = request.into_inner();

        let sb = Sandbox::get(request_data.name.as_str())
            .await
            .map_err(|err| match err {
                MicrosandboxError::SandboxNotFound(_) => {
                    Status::not_found("couldn't find specified sandbox")
                }
                _ => Status::internal("failed to get sandbox"),
            })?;
        sb.remove()
            .await
            .map_err(|_| Status::internal("failed to remove sandbox"))?;

        sync_ssh_config().await;

        Ok(Response::new(RemoveSandboxResponse {}))
    }

    /// Starts an interactive session in a sandbox and streams its input and output.
    async fn attach(
        &self,
        request: Request<Streaming<AttachRequest>>,
    ) -> Result<Response<Self::AttachStream>, Status> {
        let mut inbound = request.into_inner();

        let start = match inbound.message().await? {
            Some(AttachRequest {
                message: Some(attach_request::Message::Start(start)),
            }) => start,
            _ => return Err(Status::invalid_argument("first message must be start")),
        };

        let (width, height) = window_size(start.size)?;

        let sb = Sandbox::get(start.name.as_str())
            .await
            .map_err(|err| match err {
                MicrosandboxError::SandboxNotFound(_) => {
                    Status::not_found("couldn't find specified sandbox")
                }
                _ => Status::internal("failed to get sandbox"),
            })?
            .connect()
            .await
            .map_err(|_| Status::failed_precondition("sandbox is not running"))?;

        let mut handle = sb
            .exec_stream_with(start.command, |e| e.args(start.args).stdin_pipe().tty(true))
            .await
            .map_err(|_| Status::internal("failed to start session"))?;

        let stdin = handle
            .take_stdin()
            .ok_or_else(|| Status::internal("session has no stdin"))?;
        let control = handle.control();

        if control.resize(height, width).await.is_err() {
            end_session(&control).await;
            return Err(Status::internal("failed to resize session"));
        }

        let (tx, rx) = mpsc::channel(32);

        tokio::spawn(async move {
            run_session(inbound, handle, stdin, control, tx).await;
            // The connected handle only owns the agent connection; dropping it leaves the VM running.
            drop(sb);
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    /// Serves an SSH connection to a sandbox over the stream, starting the sandbox when needed.
    async fn ssh_tunnel(
        &self,
        request: Request<Streaming<SshTunnelRequest>>,
    ) -> Result<Response<Self::SshTunnelStream>, Status> {
        let mut inbound = request.into_inner();

        let hostname = match inbound.message().await? {
            Some(SshTunnelRequest {
                message: Some(ssh_tunnel_request::Message::Hostname(hostname)),
            }) => hostname,
            _ => return Err(Status::invalid_argument("first message must be hostname")),
        };

        let sb = Sandbox::list_with(|opt| opt.label(ssh::HOSTNAME_LABEL, &hostname))
            .await
            .map_err(|_| Status::internal("failed to list sandboxes"))?
            .sandboxes
            .into_iter()
            .next()
            .ok_or_else(|| Status::not_found("couldn't find a sandbox with that host name"))?
            .connect_or_start_detached()
            .await
            .map_err(|_| Status::failed_precondition("failed to start sandbox"))?;

        // The proxy process owns the connection, so it decides when the session ends.
        let server = sb
            .ssh()
            .server_with(|opts| {
                opts.host_key_path(ssh::host_key_path())
                    .authorized_keys_path(ssh::client_public_key_path())
                    .disable_inactivity_timeout()
            })
            .await
            .map_err(|err| {
                tracing::warn!("failed to prepare SSH server: {err}");
                Status::internal("failed to prepare SSH server")
            })?;

        let (client_end, server_end) = tokio::io::duplex(TUNNEL_BUFFER_SIZE);
        let (tx, rx) = mpsc::channel(32);

        tokio::spawn(async move {
            let serve = tokio::spawn(async move { server.serve(server_end).await });

            run_tunnel(inbound, client_end, tx).await;

            match serve.await {
                Ok(Err(err)) => tracing::info!("SSH session to {hostname} ended: {err}"),
                Err(err) => tracing::warn!("SSH server for {hostname} failed: {err}"),
                Ok(Ok(())) => {}
            }

            // The connected handle only owns the agent connection; dropping it leaves the VM running.
            drop(sb);
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }
}

/// Copies client bytes to the SSH server and server bytes to the client until the server
/// closes its end or the client goes away.
async fn run_tunnel(
    mut inbound: Streaming<SshTunnelRequest>,
    stream: DuplexStream,
    tx: mpsc::Sender<Result<SshTunnelResponse, Status>>,
) {
    let (mut reader, mut writer) = tokio::io::split(stream);

    // Both directions run concurrently so a full pipe in one direction can't stall the other.
    let upstream = async {
        loop {
            match inbound.message().await {
                Ok(Some(SshTunnelRequest {
                    message: Some(ssh_tunnel_request::Message::Data(data)),
                })) => {
                    if writer.write_all(&data).await.is_err() {
                        break;
                    }
                }
                Ok(Some(SshTunnelRequest {
                    message: Some(ssh_tunnel_request::Message::Hostname(_)),
                })) => tracing::warn!("ignoring hostname message on open tunnel"),
                Ok(Some(SshTunnelRequest { message: None })) => {}
                Ok(None) | Err(_) => break,
            }
        }

        // Signal end of input; the server closes its end in response, which ends downstream.
        let _ = writer.shutdown().await;
        std::future::pending::<()>().await;
    };

    let downstream = async {
        let mut buffer = vec![0; TUNNEL_CHUNK_SIZE];

        loop {
            match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    let response = SshTunnelResponse {
                        data: buffer[..n].to_vec(),
                    };

                    if tx.send(Ok(response)).await.is_err() {
                        return;
                    }
                }
            }
        }
    };

    tokio::select! {
        _ = upstream => {}
        _ = downstream => {}
    }
}

/// Returns every sandbox, following the list cursor across pages.
async fn list_all_sandboxes() -> Result<Vec<SandboxHandle>, Status> {
    let mut sandboxes = vec![];
    let mut next_cursor: Option<String> = None;

    loop {
        let result = Sandbox::list_with(|opt| match next_cursor.take() {
            Some(cursor) => opt.cursor(cursor),
            None => opt,
        })
        .await
        .map_err(|_| Status::internal("failed to list sandboxes"))?;

        sandboxes.extend(result.sandboxes);

        if result.next_cursor.is_none() {
            break;
        }

        next_cursor = result.next_cursor;
    }

    Ok(sandboxes)
}

/// Returns the SSH host names that sandboxes already use.
async fn taken_hostnames() -> Result<HashSet<String>, Status> {
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
            tracing::warn!("failed to update SSH config: {}", err.message());
            return;
        }
    };

    if let Err(err) = ssh::sync_config(&hostnames) {
        tracing::warn!("failed to update SSH config: {err:#}");
    }
}

/// Forwards client input to the session and session output to the client until either side ends.
async fn run_session(
    mut inbound: Streaming<AttachRequest>,
    mut handle: ExecHandle,
    stdin: ExecSink,
    control: ExecControl,
    tx: mpsc::Sender<Result<AttachResponse, Status>>,
) {
    loop {
        tokio::select! {
            message = inbound.message() => match message {
                Ok(Some(AttachRequest { message: Some(message) })) => match message {
                    attach_request::Message::Input(input) => {
                        if let Err(err) = stdin.write(input.data).await {
                            tracing::warn!("failed to write session input: {err}");
                        }
                    }
                    attach_request::Message::Resize(size) => match window_size(Some(size)) {
                        Ok((width, height)) => {
                            if let Err(err) = control.resize(height, width).await {
                                tracing::warn!("failed to resize session: {err}");
                            }
                        }
                        Err(_) => tracing::warn!("ignoring invalid resize request"),
                    },
                    attach_request::Message::Start(_) => {
                        tracing::warn!("ignoring start message on running session");
                    }
                },
                Ok(Some(AttachRequest { message: None })) => {}
                Ok(None) | Err(_) => {
                    tracing::info!("client disconnected, ending session");
                    end_session(&control).await;
                    return;
                }
            },
            event = handle.recv() => match event {
                Some(ExecEvent::Stdout(data)) | Some(ExecEvent::Stderr(data)) => {
                    let response = AttachResponse {
                        message: Some(attach_response::Message::Output(data.to_vec())),
                    };

                    if tx.send(Ok(response)).await.is_err() {
                        end_session(&control).await;
                        return;
                    }
                }
                Some(ExecEvent::Exited { code }) => {
                    let response = AttachResponse {
                        message: Some(attach_response::Message::ExitCode(code)),
                    };
                    let _ = tx.send(Ok(response)).await;
                    return;
                }
                Some(ExecEvent::Failed(failed)) => {
                    tracing::warn!("session failed to start: {failed:?}");
                    let _ = tx.send(Err(Status::internal("session failed to start"))).await;
                    return;
                }
                Some(_) => {}
                None => return,
            },
            _ = tx.closed() => {
                tracing::info!("client disconnected, ending session");
                end_session(&control).await;
                return;
            }
        }
    }
}

/// Kills the session, logging a warning when that fails.
async fn end_session(control: &ExecControl) {
    if let Err(err) = control.kill().await {
        tracing::warn!("failed to end session: {err}");
    }
}

/// Returns the guest path for a workspace: `/workspaces/<leaf-name>` of the absolute host path.
fn workspace_mount_path(workspace: &str) -> Result<String, Status> {
    let path = Path::new(workspace);

    if !path.is_absolute() {
        return Err(Status::invalid_argument(
            "workspace must be an absolute path",
        ));
    }

    let leaf = path
        .file_name()
        .ok_or_else(|| Status::invalid_argument("workspace must not be the root directory"))?;

    Ok(format!("/workspaces/{}", leaf.to_string_lossy()))
}

/// Converts a requested window size to `(width, height)` as PTY columns and rows.
fn window_size(size: Option<AttachResize>) -> Result<(u16, u16), Status> {
    let size = size.ok_or_else(|| Status::invalid_argument("window size is required"))?;

    let width = u16::try_from(size.width)
        .map_err(|_| Status::invalid_argument("window width is too large"))?;
    let height = u16::try_from(size.height)
        .map_err(|_| Status::invalid_argument("window height is too large"))?;

    Ok((width, height))
}

/// Serves the gRPC API on the socket until SIGINT or SIGTERM is received.
pub async fn run(socket_path: &Path) -> Result<(), ServerError> {
    serve(socket_path, shutdown_signal()).await
}

/// Serves the gRPC API on the socket until `shutdown` completes, then removes the socket.
pub async fn serve(
    socket_path: &Path,
    shutdown: impl Future<Output = ()>,
) -> Result<(), ServerError> {
    if socket_path.exists() {
        return Err(ServerError::SocketAlreadyInUse());
    }

    let listener = UnixListener::bind(socket_path).map_err(ServerError::InvalidSocketPath)?;
    let incoming = UnixListenerStream::new(listener);

    Server::builder()
        .trace_fn(|req| tracing::info_span!("grpc", path = %req.uri().path()))
        .add_service(SandboxManagementServiceServer::new(AnvilServer::default()))
        .serve_with_incoming_shutdown(incoming, shutdown)
        .await
        .map_err(ServerError::FailedToListen)?;

    fs::remove_file(socket_path)?;

    Ok(())
}

/// Completes when SIGINT or SIGTERM is received.
async fn shutdown_signal() {
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => tracing::info!("shutting down"),
        _ = sigterm.recv() => tracing::info!("shutting down"),
    }
}

/// Converts a microsandbox status to its protobuf status value.
fn map_sandbox_status(s: microsandbox::sandbox::SandboxStatus) -> i32 {
    let status = match s {
        microsandbox::sandbox::SandboxStatus::Running => SandboxStatus::Running,
        microsandbox::sandbox::SandboxStatus::Stopped => SandboxStatus::Stopped,
        microsandbox::sandbox::SandboxStatus::Created => SandboxStatus::Stopped,
        microsandbox::sandbox::SandboxStatus::Starting => SandboxStatus::Starting,
        microsandbox::sandbox::SandboxStatus::Paused => SandboxStatus::Paused,
        microsandbox::sandbox::SandboxStatus::Draining => SandboxStatus::Stopping,
        microsandbox::sandbox::SandboxStatus::Crashed => SandboxStatus::Crashed,
    };

    status.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use microsandbox::sandbox::SandboxStatus as MsbStatus;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("anvil-test-{}-{}", std::process::id(), name))
    }

    #[test]
    fn map_sandbox_status_maps_every_variant() {
        let cases = [
            (MsbStatus::Running, SandboxStatus::Running),
            (MsbStatus::Stopped, SandboxStatus::Stopped),
            (MsbStatus::Created, SandboxStatus::Stopped),
            (MsbStatus::Starting, SandboxStatus::Starting),
            (MsbStatus::Paused, SandboxStatus::Paused),
            (MsbStatus::Draining, SandboxStatus::Stopping),
            (MsbStatus::Crashed, SandboxStatus::Crashed),
        ];

        for (input, expected) in cases {
            assert_eq!(map_sandbox_status(input), expected as i32);
        }
    }

    #[test]
    fn map_sandbox_status_produces_valid_proto_values() {
        let value = map_sandbox_status(MsbStatus::Draining);
        assert_eq!(SandboxStatus::try_from(value), Ok(SandboxStatus::Stopping));
    }

    #[test]
    fn window_size_converts_to_width_and_height() {
        let size = AttachResize {
            width: 120,
            height: 40,
        };

        assert_eq!(window_size(Some(size)).unwrap(), (120, 40));
    }

    #[test]
    fn window_size_rejects_missing_and_oversized_values() {
        let oversized = AttachResize {
            width: u32::from(u16::MAX) + 1,
            height: 40,
        };

        assert_eq!(
            window_size(None).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
        assert_eq!(
            window_size(Some(oversized)).unwrap_err().code(),
            tonic::Code::InvalidArgument
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
            assert_eq!(
                workspace_mount_path(workspace).unwrap_err().code(),
                tonic::Code::InvalidArgument,
                "{workspace:?} should be rejected"
            );
        }
    }

    #[tokio::test]
    async fn run_fails_when_socket_already_exists() {
        let path = temp_path("existing.sock");
        fs::write(&path, b"").unwrap();

        let result = run(&path).await;

        fs::remove_file(&path).unwrap();
        assert!(matches!(result, Err(ServerError::SocketAlreadyInUse())));
    }

    #[tokio::test]
    async fn serve_removes_socket_on_shutdown() {
        let path = temp_path("shutdown.sock");
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        let server_path = path.clone();
        let handle = tokio::spawn(async move {
            serve(&server_path, async {
                let _ = rx.await;
            })
            .await
        });

        while tokio::net::UnixStream::connect(&path).await.is_err() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        tx.send(()).unwrap();
        handle.await.unwrap().unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn run_fails_when_socket_directory_does_not_exist() {
        let path = temp_path("missing-dir").join("anvil.sock");

        let result = run(&path).await;

        assert!(matches!(result, Err(ServerError::InvalidSocketPath(_))));
        assert!(!path.exists());
    }
}

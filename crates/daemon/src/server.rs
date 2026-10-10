//! The gRPC server: adapts `SandboxManagementService` requests to the sandbox management in
//! [`crate::sandboxes`], [`crate::session`] and [`crate::tunnel`].

use crate::api::sandbox_management_service_server::{
    SandboxManagementService, SandboxManagementServiceServer,
};
use crate::api::{
    AttachRequest, AttachResponse, AttachStart, GetSandboxRequest, GetSandboxResponse,
    ListSandboxesRequest, ListSandboxesResponse, ListSecretsRequest, ListSecretsResponse,
    RemoveSandboxRequest, RemoveSandboxResponse, RemoveSecretRequest, RemoveSecretResponse,
    SandboxResources, SandboxStatus, SandboxSummary, SecretSummary, SetSecretRequest,
    SetSecretResponse, SshTunnelRequest, SshTunnelResponse, StartSandboxRequest,
    StartSandboxResponse, StopSandboxRequest, StopSandboxResponse, attach_request,
    ssh_tunnel_request,
};
use crate::sandboxes::{Resources, SandboxError, SandboxInfo, SandboxManager, StartSandbox};
use crate::secrets::{Secret, SecretStore};
use crate::session::{self, SessionCommand};
use crate::tunnel;
use anyhow::Result;
use async_trait::async_trait;
use firebrick_spec::{SandboxResourcesSpec, VolumesSpec};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::pin::Pin;
use thiserror::Error;
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tonic::codegen::tokio_stream::wrappers::UnixListenerStream;
use tonic::codegen::tokio_stream::{Stream, StreamExt};
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
    #[error("can't restrict the socket to its owner")]
    InsecureSocket(#[source] std::io::Error),
}

/// gRPC service implementation that manages sandboxes.
pub struct FirebrickServer {
    sandboxes: SandboxManager,
}

impl FirebrickServer {
    /// Creates a server that adds the secrets from `secrets` to sandboxes.
    pub fn new(secrets: SecretStore) -> Self {
        Self {
            sandboxes: SandboxManager::new(secrets),
        }
    }
}

/// Reads the sandbox to start from a request. A request without `init` runs the image's init.
fn start_sandbox_from(request: &StartSandboxRequest) -> StartSandbox<'_> {
    StartSandbox {
        name: &request.name,
        workspace: &request.workspace,
        image: &request.image,
        init: request.init.unwrap_or(true),
    }
}

impl From<SandboxError> for Status {
    fn from(err: SandboxError) -> Self {
        match err {
            SandboxError::NotFound(message) => Status::not_found(message),
            SandboxError::InvalidArgument(message) => Status::invalid_argument(message),
            SandboxError::FailedPrecondition(message) => Status::failed_precondition(message),
            SandboxError::Internal(message) => Status::internal(message),
        }
    }
}

/// Stream of responses sent back to the client during an attached session.
type AttachStream = Pin<Box<dyn Stream<Item = Result<AttachResponse, Status>> + Send>>;

/// Stream of SSH protocol bytes sent back to the client through a tunnel.
type SshTunnelStream = Pin<Box<dyn Stream<Item = Result<SshTunnelResponse, Status>> + Send>>;

#[async_trait]
impl SandboxManagementService for FirebrickServer {
    type AttachStream = AttachStream;
    type SshTunnelStream = SshTunnelStream;

    /// Starts an existing sandbox or creates a new one when it doesn't exist.
    async fn start_sandbox(
        &self,
        request: Request<StartSandboxRequest>,
    ) -> Result<Response<StartSandboxResponse>, Status> {
        let request_data = request.into_inner();

        self.sandboxes
            .start(start_sandbox_from(&request_data), || {
                sandbox_resources(&request_data)
            })
            .await?;

        Ok(Response::new(StartSandboxResponse {}))
    }

    /// Stores a secret and adds it to the existing sandboxes. Running sandboxes pick it up the
    /// next time they start.
    async fn set_secret(
        &self,
        request: Request<SetSecretRequest>,
    ) -> Result<Response<SetSecretResponse>, Status> {
        let request_data = request.into_inner();
        let failed_sandboxes = self
            .sandboxes
            .set_secret(
                request_data.name,
                request_data.value,
                request_data.allowed_hosts,
            )
            .await?;

        Ok(Response::new(SetSecretResponse { failed_sandboxes }))
    }

    /// Lists the stored secrets by name, without their values.
    async fn list_secrets(
        &self,
        _request: Request<ListSecretsRequest>,
    ) -> Result<Response<ListSecretsResponse>, Status> {
        let secrets = self
            .sandboxes
            .list_secrets()?
            .iter()
            .map(secret_summary)
            .collect();

        Ok(Response::new(ListSecretsResponse { secrets }))
    }

    /// Removes a stored secret and removes it from the existing sandboxes. Running sandboxes
    /// keep it until they restart.
    async fn remove_secret(
        &self,
        request: Request<RemoveSecretRequest>,
    ) -> Result<Response<RemoveSecretResponse>, Status> {
        let name = request.into_inner().name;
        let failed_sandboxes = self.sandboxes.remove_secret(&name).await?;

        Ok(Response::new(RemoveSecretResponse { failed_sandboxes }))
    }

    /// Stops a running sandbox.
    async fn stop_sandbox(
        &self,
        request: Request<StopSandboxRequest>,
    ) -> Result<Response<StopSandboxResponse>, Status> {
        self.sandboxes.stop(&request.into_inner().name).await?;

        Ok(Response::new(StopSandboxResponse {}))
    }

    /// Lists all sandboxes with their status.
    async fn list_sandboxes(
        &self,
        _request: Request<ListSandboxesRequest>,
    ) -> Result<Response<ListSandboxesResponse>, Status> {
        let sandboxes = self
            .sandboxes
            .list()
            .await?
            .into_iter()
            .map(sandbox_summary)
            .collect();

        Ok(Response::new(ListSandboxesResponse { sandboxes }))
    }

    /// Returns the name, status, host name and workspace path of a single sandbox.
    async fn get_sandbox(
        &self,
        request: Request<GetSandboxRequest>,
    ) -> Result<Response<GetSandboxResponse>, Status> {
        let sandbox = self.sandboxes.get(&request.into_inner().name).await?;

        Ok(Response::new(GetSandboxResponse {
            name: sandbox.name,
            status: map_sandbox_status(sandbox.status),
            hostname: sandbox.hostname.unwrap_or_default(),
            workspace_path: sandbox.workspace_path.unwrap_or_default(),
        }))
    }

    /// Removes a sandbox, stopping it first when the request forces it.
    async fn remove_sandbox(
        &self,
        request: Request<RemoveSandboxRequest>,
    ) -> std::result::Result<Response<RemoveSandboxResponse>, Status> {
        let request = request.into_inner();
        self.sandboxes.remove(&request.name, request.force).await?;

        Ok(Response::new(RemoveSandboxResponse {}))
    }

    /// Starts an interactive session in a sandbox and streams its input and output.
    async fn attach(
        &self,
        request: Request<Streaming<AttachRequest>>,
    ) -> Result<Response<Self::AttachStream>, Status> {
        let mut inbound = request.into_inner();
        let start = attach_start(&mut inbound).await?;
        let size = session::window_size(start.size)?;
        let sb = self.sandboxes.connect(&start.name).await?;

        let command = SessionCommand {
            command: start.command,
            args: start.args,
            size,
        };
        let outbound = session::attach(sb, inbound, command).await?;

        Ok(Response::new(Box::pin(outbound)))
    }

    /// Serves an SSH connection to a sandbox over the stream, starting the sandbox when needed.
    async fn ssh_tunnel(
        &self,
        request: Request<Streaming<SshTunnelRequest>>,
    ) -> Result<Response<Self::SshTunnelStream>, Status> {
        let mut inbound = request.into_inner();
        let hostname = tunnel_hostname(&mut inbound).await?;
        let sb = self.sandboxes.connect_by_hostname(&hostname).await?;
        let outbound = tunnel::open(sb, inbound, hostname).await?;

        Ok(Response::new(Box::pin(outbound)))
    }
}

/// Returns the start message, which must be the first message of an `Attach` stream.
async fn attach_start(inbound: &mut Streaming<AttachRequest>) -> Result<AttachStart, Status> {
    match inbound.message().await? {
        Some(AttachRequest {
            message: Some(attach_request::Message::Start(start)),
        }) => Ok(start),
        _ => Err(Status::invalid_argument("first message must be start")),
    }
}

/// Returns the host name, which must be the first message of an `SshTunnel` stream.
async fn tunnel_hostname(inbound: &mut Streaming<SshTunnelRequest>) -> Result<String, Status> {
    match inbound.message().await? {
        Some(SshTunnelRequest {
            message: Some(ssh_tunnel_request::Message::Hostname(hostname)),
        }) => Ok(hostname),
        _ => Err(Status::invalid_argument("first message must be hostname")),
    }
}

/// Converts a sandbox to its protobuf summary.
fn sandbox_summary(sandbox: SandboxInfo) -> SandboxSummary {
    SandboxSummary {
        name: sandbox.name,
        status: map_sandbox_status(sandbox.status),
        hostname: sandbox.hostname.unwrap_or_default(),
    }
}

/// Converts a secret to its protobuf summary, without its value.
fn secret_summary(secret: &Secret) -> SecretSummary {
    SecretSummary {
        name: secret.name().to_string(),
        allowed_hosts: secret.allowed_hosts().to_vec(),
    }
}

/// Converts the requested resources and volumes to vCPUs and MiB, using the defaults for
/// missing resources, missing volumes and an empty volume size.
fn sandbox_resources(request: &StartSandboxRequest) -> Result<Resources, SandboxError> {
    let resources = request.resources.clone().unwrap_or_else(|| {
        let defaults = SandboxResourcesSpec::default();

        SandboxResources {
            cpu: defaults.cpu.into(),
            memory: defaults.memory,
        }
    });
    let docker_volume = request
        .volumes
        .as_ref()
        .map(|volumes| volumes.docker.clone())
        .filter(|size| !size.is_empty())
        .unwrap_or_else(|| VolumesSpec::default().docker);

    let cpus = u8::try_from(resources.cpu)
        .ok()
        .filter(|cpus| *cpus > 0)
        .ok_or_else(|| SandboxError::InvalidArgument("cpu must be between 1 and 255".into()))?;

    Ok(Resources {
        cpus,
        memory_mib: parse_size_mib(&resources.memory)?,
        docker_volume_mib: parse_size_mib(&docker_volume)?,
    })
}

/// Parses a requested size into MiB, reporting a bad size as an invalid argument.
fn parse_size_mib(size: &str) -> Result<u32, SandboxError> {
    firebrick_spec::parse_size_mib(size)
        .map_err(|err| SandboxError::InvalidArgument(err.to_string()))
}

/// Serves the gRPC API on the socket until SIGINT or SIGTERM is received.
pub async fn run(socket_path: &Path, secrets: SecretStore) -> Result<(), ServerError> {
    serve(socket_path, secrets, shutdown_signal()).await
}

/// Serves the gRPC API on the socket until `shutdown` completes, then removes the socket.
pub async fn serve(
    socket_path: &Path,
    secrets: SecretStore,
    shutdown: impl Future<Output = ()>,
) -> Result<(), ServerError> {
    if socket_path.exists() {
        return Err(ServerError::SocketAlreadyInUse());
    }

    let listener = UnixListener::bind(socket_path).map_err(ServerError::InvalidSocketPath)?;
    let daemon_uid = restrict_to_owner(socket_path).inspect_err(|_| {
        let _ = fs::remove_file(socket_path);
    })?;
    let incoming = UnixListenerStream::new(listener)
        .filter(move |conn| conn.as_ref().map_or(true, |s| accept(s, daemon_uid)));

    Server::builder()
        .trace_fn(|req| tracing::info_span!("grpc", path = %req.uri().path()))
        .add_service(SandboxManagementServiceServer::new(FirebrickServer::new(
            secrets,
        )))
        .serve_with_incoming_shutdown(incoming, shutdown)
        .await
        .map_err(ServerError::FailedToListen)?;

    fs::remove_file(socket_path)?;

    Ok(())
}

/// Gives the socket mode `0600`, whatever the umask, and returns the UID that owns it, which
/// is the daemon's effective UID.
fn restrict_to_owner(socket_path: &Path) -> Result<u32, ServerError> {
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600))
        .and_then(|()| fs::metadata(socket_path))
        .map(|metadata| metadata.uid())
        .map_err(ServerError::InsecureSocket)
}

/// Returns whether a connection may reach the gRPC service, based on its peer credentials.
/// A rejected connection is closed when the caller drops it.
fn accept(conn: &UnixStream, daemon_uid: u32) -> bool {
    match conn.peer_cred() {
        Ok(peer) if is_authorized(peer.uid(), daemon_uid) => true,
        Ok(peer) => {
            tracing::warn!(
                peer_uid = peer.uid(),
                peer_pid = ?peer.pid(),
                "rejected connection from another user"
            );
            false
        }
        Err(err) => {
            tracing::warn!(error = ?err, "rejected connection with unknown peer credentials");
            false
        }
    }
}

/// Returns whether a peer may use the daemon: only the daemon's own user and root may. Root is
/// allowed because it can bypass the check anyway, for example by reading the daemon's memory.
fn is_authorized(peer_uid: u32, daemon_uid: u32) -> bool {
    peer_uid == daemon_uid || peer_uid == 0
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
    use crate::api::SandboxVolumes;
    use crate::api::sandbox_management_service_client::SandboxManagementServiceClient;
    use hyper_util::rt::TokioIo;
    use microsandbox::sandbox::SandboxStatus as MsbStatus;
    use std::path::PathBuf;
    use tonic::transport::{Channel, Endpoint, Uri};

    /// Returns a store for tests that never set a secret.
    fn unused_store() -> SecretStore {
        SecretStore::new(temp_path("unused-secrets.yml"))
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("firebrick-test-{}-{}", std::process::id(), name))
    }

    /// Returns a store at `path` with a secret for `example.com` under each name.
    fn store_with_secrets(path: &Path, names: &[&str]) -> SecretStore {
        let store = SecretStore::new(path);
        let secrets = names
            .iter()
            .map(|name| Secret::new(name.to_string(), "value".into(), vec!["example.com".into()]));

        for secret in secrets {
            store.set(secret.unwrap()).unwrap();
        }

        store
    }

    /// Serves on the socket until `stop` fires.
    async fn serve_until(
        path: PathBuf,
        stop: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<(), ServerError> {
        serve(&path, unused_store(), async {
            let _ = stop.await;
        })
        .await
    }

    /// Waits until the server accepts connections on the socket.
    async fn wait_for_socket(path: &Path) {
        while UnixStream::connect(path).await.is_err() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    /// Connects a gRPC client to the server on the socket.
    async fn client(path: PathBuf) -> SandboxManagementServiceClient<Channel> {
        // The HTTP endpoint isn't used; it only shows up in the authority header.
        let channel = Endpoint::try_from("http://localhost")
            .unwrap()
            .connect_with_connector(tower::service_fn(move |_: Uri| connect_unix(path.clone())))
            .await
            .unwrap();

        SandboxManagementServiceClient::new(channel)
    }

    /// Connects to the server socket for a gRPC channel.
    async fn connect_unix(path: PathBuf) -> std::io::Result<TokioIo<UnixStream>> {
        Ok(TokioIo::new(UnixStream::connect(path).await?))
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
    fn start_sandbox_from_request_defaults_init_to_true() {
        let request = StartSandboxRequest::default();

        assert!(start_sandbox_from(&request).init);
    }

    #[test]
    fn start_sandbox_from_request_keeps_init() {
        let request = StartSandboxRequest {
            name: "dev".to_string(),
            image: "alpine:3.22".to_string(),
            workspace: "/home/user/project".to_string(),
            init: Some(false),
            ..Default::default()
        };

        let start = start_sandbox_from(&request);

        assert_eq!(
            (start.name, start.image, start.workspace, start.init),
            ("dev", "alpine:3.22", "/home/user/project", false)
        );
    }

    #[test]
    fn sandbox_errors_map_to_status_codes_and_messages() {
        let cases = [
            (SandboxError::NotFound("a".into()), tonic::Code::NotFound),
            (
                SandboxError::InvalidArgument("b".into()),
                tonic::Code::InvalidArgument,
            ),
            (
                SandboxError::FailedPrecondition("c".into()),
                tonic::Code::FailedPrecondition,
            ),
            (SandboxError::Internal("d".into()), tonic::Code::Internal),
        ];

        for (err, code) in cases {
            let message = err.to_string();
            let status = Status::from(err);

            assert_eq!(status.code(), code);
            assert_eq!(status.message(), message);
        }
    }

    /// Returns a start request with the resources and Docker volume size.
    fn resources_request(cpu: u32, memory: &str, docker: Option<&str>) -> StartSandboxRequest {
        StartSandboxRequest {
            resources: Some(SandboxResources {
                cpu,
                memory: memory.to_string(),
            }),
            volumes: docker.map(|docker| SandboxVolumes {
                docker: docker.to_string(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn sandbox_resources_converts_request() {
        let request = resources_request(4, "8 GiB", Some("40 GiB"));

        assert_eq!(
            sandbox_resources(&request).unwrap(),
            Resources {
                cpus: 4,
                memory_mib: 8192,
                docker_volume_mib: 40960,
            }
        );
    }

    #[test]
    fn sandbox_resources_uses_default_docker_volume_when_missing_or_empty() {
        for docker in [None, Some("")] {
            let request = resources_request(4, "8 GiB", docker);

            assert_eq!(
                sandbox_resources(&request).unwrap().docker_volume_mib,
                20480,
                "{docker:?}"
            );
        }
    }

    #[test]
    fn sandbox_resources_falls_back_to_defaults() {
        assert_eq!(
            sandbox_resources(&StartSandboxRequest::default()).unwrap(),
            Resources {
                cpus: 2,
                memory_mib: 4096,
                docker_volume_mib: 20480,
            }
        );
    }

    #[test]
    fn sandbox_resources_rejects_invalid_values() {
        let cases = [
            (0, "1 GiB", None),
            (256, "1 GiB", None),
            (2, "lots", None),
            (2, "", None),
            (2, "1 GiB", Some("20 GB")),
            (2, "1 GiB", Some("0 GiB")),
        ];

        for (cpu, memory, docker) in cases {
            let request = resources_request(cpu, memory, docker);

            assert_eq!(
                Status::from(sandbox_resources(&request).unwrap_err()).code(),
                tonic::Code::InvalidArgument,
                "cpu {cpu}, memory {memory:?}, docker {docker:?} should be rejected"
            );
        }
    }

    #[tokio::test]
    async fn run_fails_when_socket_already_exists() {
        let path = temp_path("existing.sock");
        fs::write(&path, b"").unwrap();

        let result = run(&path, unused_store()).await;

        fs::remove_file(&path).unwrap();
        assert!(matches!(result, Err(ServerError::SocketAlreadyInUse())));
    }

    #[tokio::test]
    async fn serve_removes_socket_on_shutdown() {
        let path = temp_path("shutdown.sock");
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        let handle = tokio::spawn(serve_until(path.clone(), rx));
        wait_for_socket(&path).await;

        tx.send(()).unwrap();
        handle.await.unwrap().unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn serve_restricts_socket_to_owner() {
        let path = temp_path("mode.sock");
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        let handle = tokio::spawn(serve_until(path.clone(), rx));
        wait_for_socket(&path).await;
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;

        tx.send(()).unwrap();
        handle.await.unwrap().unwrap();
        assert_eq!(mode, 0o600);
    }

    #[tokio::test]
    async fn serve_serves_connections_from_the_same_user() {
        let path = temp_path("same-user.sock");
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        let handle = tokio::spawn(serve_until(path.clone(), rx));
        wait_for_socket(&path).await;
        let result = client(path.clone())
            .await
            .list_secrets(ListSecretsRequest {})
            .await;

        tx.send(()).unwrap();
        handle.await.unwrap().unwrap();
        assert!(result.is_ok(), "request failed: {result:?}");
    }

    #[test]
    fn is_authorized_accepts_the_daemon_user_and_root_only() {
        let cases = [(1000, true), (0, true), (1001, false), (u32::MAX, false)];

        for (peer_uid, expected) in cases {
            assert_eq!(
                is_authorized(peer_uid, 1000),
                expected,
                "peer uid {peer_uid}"
            );
        }
    }

    #[tokio::test]
    async fn run_fails_when_socket_directory_does_not_exist() {
        let path = temp_path("missing-dir").join("firebrick.sock");

        let result = run(&path, unused_store()).await;

        assert!(matches!(result, Err(ServerError::InvalidSocketPath(_))));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn set_secret_rejects_invalid_secret_without_storing_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.yml");
        let server = FirebrickServer::new(SecretStore::new(&path));

        let status = server
            .set_secret(Request::new(SetSecretRequest {
                name: "NOT-A-NAME".to_string(),
                value: "value".to_string(),
                allowed_hosts: vec![],
            }))
            .await
            .expect_err("an invalid name should be rejected");

        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn list_secrets_returns_names_and_hosts_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_secrets(&dir.path().join("secrets.yml"), &["B_TOKEN", "A_TOKEN"]);
        let server = FirebrickServer::new(store);

        let secrets = server
            .list_secrets(Request::new(ListSecretsRequest {}))
            .await
            .unwrap()
            .into_inner()
            .secrets;

        let expected: Vec<SecretSummary> = ["A_TOKEN", "B_TOKEN"]
            .map(|name| SecretSummary {
                name: name.to_string(),
                allowed_hosts: vec!["example.com".to_string()],
            })
            .into();
        assert_eq!(secrets, expected);
    }

    #[tokio::test]
    async fn remove_secret_rejects_invalid_name() {
        let server = FirebrickServer::new(unused_store());

        let status = server
            .remove_secret(Request::new(RemoveSecretRequest {
                name: "NOT-A-NAME".to_string(),
            }))
            .await
            .expect_err("an invalid name should be rejected");

        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn remove_secret_returns_not_found_for_unknown_secret() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.yml");
        let store = SecretStore::new(&path);
        let secret = Secret::new("A".into(), "value".into(), vec!["example.com".into()]).unwrap();
        store.set(secret).unwrap();
        let server = FirebrickServer::new(store);

        let status = server
            .remove_secret(Request::new(RemoveSecretRequest {
                name: "B".to_string(),
            }))
            .await
            .expect_err("removing an unknown secret should fail");

        assert_eq!(status.code(), tonic::Code::NotFound);
        assert_eq!(SecretStore::new(&path).load().unwrap().len(), 1);
    }
}

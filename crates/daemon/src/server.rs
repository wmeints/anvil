use crate::api::sandbox_management_service_server::{
    SandboxManagementService, SandboxManagementServiceServer,
};
use crate::api::{
    AttachRequest, AttachResize, AttachResponse, GetSandboxRequest, GetSandboxResponse,
    ListSandboxesRequest, ListSandboxesResponse, ListSecretsRequest, ListSecretsResponse,
    RemoveSandboxRequest, RemoveSandboxResponse, RemoveSecretRequest, RemoveSecretResponse,
    SandboxResources, SandboxStatus, SandboxSummary, SecretSummary, SetSecretRequest,
    SetSecretResponse, SshTunnelRequest, SshTunnelResponse, StartSandboxRequest,
    StartSandboxResponse, StopSandboxRequest, StopSandboxResponse, attach_request, attach_response,
    ssh_tunnel_request,
};
use crate::secrets::{self, Secret, SecretStore};
use crate::ssh;
use anvil_spec::SandboxResourcesSpec;
use anyhow::Result;
use async_trait::async_trait;
use microsandbox::sandbox::exec::{ExecControl, ExecEvent, ExecHandle, ExecSink};
use microsandbox::sandbox::ssh::SshServer;
use microsandbox::sandbox::{HostPermissions, SandboxHandle};
use microsandbox::{MicrosandboxError, Sandbox};
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::ops::ControlFlow;
use std::path::Path;
use std::pin::Pin;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};
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
pub struct AnvilServer {
    secrets: SecretStore,
    // Held while secrets are stored or added to sandboxes, so a sandbox that is being created
    // can't miss a secret that is being set, and concurrent sets can't mix up values.
    secrets_lock: tokio::sync::Mutex<()>,
}

impl AnvilServer {
    /// Creates a server that adds the secrets from `secrets` to sandboxes.
    pub fn new(secrets: SecretStore) -> Self {
        Self {
            secrets,
            secrets_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Returns the stored secrets.
    fn load_secrets(&self) -> Result<Vec<Secret>, Status> {
        self.secrets.load().map_err(|err| {
            tracing::warn!("failed to load secrets: {err:#}");
            Status::internal("failed to load secrets")
        })
    }

    /// Fails with `NotFound` when no secret with the name is stored.
    fn ensure_secret_exists(&self, name: &str) -> Result<(), Status> {
        if self
            .load_secrets()?
            .iter()
            .any(|secret| secret.name() == name)
        {
            Ok(())
        } else {
            Err(Status::not_found(format!("secret {name} doesn't exist")))
        }
    }

    /// Creates a sandbox for the workspace with the stored secrets.
    async fn create_sandbox(&self, request: &StartSandboxRequest) -> Result<(), Status> {
        let guest_path = workspace_mount_path(&request.workspace)?;
        let (cpus, memory_mib) = sandbox_resources(request.resources.clone())?;
        let hostname = ssh::pick_hostname(&request.workspace, &taken_hostnames().await?);
        let _secrets_guard = self.secrets_lock.lock().await;
        let secrets = self.load_secrets()?;

        let builder = Sandbox::builder(&request.name)
            .image(sandbox_image(&request.image))
            .cpus(cpus)
            .memory(memory_mib)
            .label(ssh::HOSTNAME_LABEL, &hostname)
            // Mounted read/write; Mirror propagates guest chmod changes to the host files.
            .volume(&guest_path, |m| {
                m.bind(&request.workspace)
                    .host_permissions(HostPermissions::Mirror)
            })
            .workdir(&guest_path)
            .detached(true);

        secrets::add_to_builder(builder, &secrets)
            .create()
            .await
            .map_err(|err| {
                tracing::warn!("failed to create sandbox {}: {err}", request.name);
                Status::internal("failed to create sandbox")
            })?;

        tracing::info!("created sandbox {} as {hostname}", request.name);

        Ok(())
    }
}

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

        match Sandbox::get(&request_data.name).await {
            Ok(existing_sb) => start_existing_sandbox(&existing_sb, &request_data).await?,
            Err(_) => self.create_sandbox(&request_data).await?,
        }

        sync_ssh_config().await;

        Ok(Response::new(StartSandboxResponse {}))
    }

    /// Stores a secret and adds it to the existing sandboxes. Running sandboxes pick it up the
    /// next time they start.
    async fn set_secret(
        &self,
        request: Request<SetSecretRequest>,
    ) -> Result<Response<SetSecretResponse>, Status> {
        let request_data = request.into_inner();
        let secret = Secret::new(
            request_data.name,
            request_data.value,
            request_data.allowed_hosts,
        )
        .map_err(|err| Status::invalid_argument(err.to_string()))?;

        let _secrets_guard = self.secrets_lock.lock().await;

        self.secrets.set(secret.clone()).map_err(|err| {
            tracing::warn!("failed to store secret {}: {err:#}", secret.name());
            Status::internal("failed to store secret")
        })?;

        let failed_sandboxes = update_anvil_sandboxes(SecretChange::Add(&secret))
            .await
            .map_err(|_| {
                Status::internal("stored the secret, but failed to list the sandboxes to add it to")
            })?;

        tracing::info!("set secret {}", secret.name());

        Ok(Response::new(SetSecretResponse { failed_sandboxes }))
    }

    /// Lists the stored secrets by name, without their values.
    async fn list_secrets(
        &self,
        _request: Request<ListSecretsRequest>,
    ) -> Result<Response<ListSecretsResponse>, Status> {
        let mut secrets: Vec<SecretSummary> = self
            .load_secrets()?
            .iter()
            .map(|secret| SecretSummary {
                name: secret.name().to_string(),
                allowed_hosts: secret.allowed_hosts().to_vec(),
            })
            .collect();

        secrets.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(Response::new(ListSecretsResponse { secrets }))
    }

    /// Removes a stored secret and removes it from the existing sandboxes. Running sandboxes
    /// keep it until they restart.
    async fn remove_secret(
        &self,
        request: Request<RemoveSecretRequest>,
    ) -> Result<Response<RemoveSecretResponse>, Status> {
        let name = request.into_inner().name;
        secrets::validate_name(&name).map_err(|err| Status::invalid_argument(err.to_string()))?;

        let _secrets_guard = self.secrets_lock.lock().await;
        self.ensure_secret_exists(&name)?;

        // Remove the secret from the store last, so it can be removed again when a sandbox
        // fails.
        let failed_sandboxes = update_anvil_sandboxes(SecretChange::Remove(&name)).await?;

        if !failed_sandboxes.is_empty() {
            return Ok(Response::new(RemoveSecretResponse { failed_sandboxes }));
        }

        self.secrets.remove(&name).map_err(|err| {
            tracing::warn!("failed to remove secret {name}: {err:#}");
            Status::internal("failed to remove secret")
        })?;

        tracing::info!("removed secret {name}");

        Ok(Response::new(RemoveSecretResponse { failed_sandboxes }))
    }

    /// Stops a running sandbox.
    async fn stop_sandbox(
        &self,
        request: Request<StopSandboxRequest>,
    ) -> Result<Response<StopSandboxResponse>, Status> {
        let sb = get_sandbox(&request.into_inner().name).await?;

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
        let sb = get_sandbox(&request.into_inner().name).await?;

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
        let sb = get_sandbox(&request.into_inner().name).await?;

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

        let sb = get_sandbox(&start.name)
            .await?
            .connect()
            .await
            .map_err(|_| Status::failed_precondition("sandbox is not running"))?;

        let session = Session::open(&sb, start.command, start.args).await?;
        session.resize(width, height).await?;

        let (tx, rx) = mpsc::channel(32);

        tokio::spawn(async move {
            run_session(inbound, session, tx).await;
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

        let sb = connect_by_hostname(&hostname).await?;
        let server = ssh_server(&sb).await?;
        let (tx, rx) = mpsc::channel(32);

        tokio::spawn(async move {
            serve_tunnel(server, inbound, tx, &hostname).await;
            // The connected handle only owns the agent connection; dropping it leaves the VM running.
            drop(sb);
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }
}

/// Returns the sandbox with the name.
async fn get_sandbox(name: &str) -> Result<SandboxHandle, Status> {
    Sandbox::get(name).await.map_err(|err| match err {
        MicrosandboxError::SandboxNotFound(_) => {
            Status::not_found("couldn't find specified sandbox")
        }
        _ => Status::internal("failed to get sandbox"),
    })
}

/// Starts an existing sandbox, first giving it a host name when it has none.
async fn start_existing_sandbox(
    sb: &SandboxHandle,
    request: &StartSandboxRequest,
) -> Result<(), Status> {
    // Sandboxes created before SSH support have no host name yet.
    if ssh::hostname_of(sb).is_none() {
        assign_hostname(sb, request).await?;
    }

    sb.start_detached()
        .await
        .map_err(|_| Status::internal("failed to start sandbox"))?;

    tracing::info!("started sandbox {}", request.name);

    Ok(())
}

/// Gives the sandbox a host name based on its workspace, logging a warning when that fails.
/// Fails when the host names other sandboxes use can't be listed.
async fn assign_hostname(sb: &SandboxHandle, request: &StartSandboxRequest) -> Result<(), Status> {
    let workspace = match request.workspace.as_str() {
        "" => request.name.as_str(),
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

/// Connects to the sandbox with the SSH host name, starting it when needed.
async fn connect_by_hostname(hostname: &str) -> Result<Sandbox, Status> {
    Sandbox::list_with(|opt| opt.label(ssh::HOSTNAME_LABEL, hostname))
        .await
        .map_err(|_| Status::internal("failed to list sandboxes"))?
        .sandboxes
        .into_iter()
        .next()
        .ok_or_else(|| Status::not_found("couldn't find a sandbox with that host name"))?
        .connect_or_start_detached()
        .await
        .map_err(|_| Status::failed_precondition("failed to start sandbox"))
}

/// Prepares the SSH server of a sandbox for a tunnel.
async fn ssh_server(sb: &Sandbox) -> Result<SshServer, Status> {
    // The proxy process owns the connection, so it decides when the session ends.
    sb.ssh()
        .server_with(|opts| {
            opts.host_key_path(ssh::host_key_path())
                .authorized_keys_path(ssh::client_public_key_path())
                .disable_inactivity_timeout()
        })
        .await
        .map_err(|err| {
            tracing::warn!("failed to prepare SSH server: {err}");
            Status::internal("failed to prepare SSH server")
        })
}

/// Serves an SSH session through the tunnel until it closes.
async fn serve_tunnel(
    server: SshServer,
    inbound: Streaming<SshTunnelRequest>,
    tx: mpsc::Sender<Result<SshTunnelResponse, Status>>,
    hostname: &str,
) {
    let (client_end, server_end) = tokio::io::duplex(TUNNEL_BUFFER_SIZE);
    let serve = tokio::spawn(async move { server.serve(server_end).await });

    run_tunnel(inbound, client_end, tx).await;

    match serve.await {
        Ok(Err(err)) => tracing::info!("SSH session to {hostname} ended: {err}"),
        Err(err) => tracing::warn!("SSH server for {hostname} failed: {err}"),
        Ok(Ok(())) => {}
    }
}

/// Copies client bytes to the SSH server and server bytes to the client until the server
/// closes its end or the client goes away.
async fn run_tunnel(
    inbound: Streaming<SshTunnelRequest>,
    stream: DuplexStream,
    tx: mpsc::Sender<Result<SshTunnelResponse, Status>>,
) {
    let (reader, writer) = tokio::io::split(stream);

    // The server closes its end in response to the end of input, which ends downstream.
    let upstream = async {
        copy_upstream(inbound, writer).await;
        std::future::pending::<()>().await;
    };

    // Both directions run concurrently so a full pipe in one direction can't stall the other.
    tokio::select! {
        _ = upstream => {}
        _ = copy_downstream(reader, &tx) => {}
    }
}

/// Copies client bytes to the SSH server, then signals the end of input.
async fn copy_upstream(
    mut inbound: Streaming<SshTunnelRequest>,
    mut writer: WriteHalf<DuplexStream>,
) {
    while let Some(data) = next_tunnel_data(&mut inbound).await {
        if writer.write_all(&data).await.is_err() {
            break;
        }
    }

    let _ = writer.shutdown().await;
}

/// Returns the next bytes the client sends, or `None` when the client goes away.
async fn next_tunnel_data(inbound: &mut Streaming<SshTunnelRequest>) -> Option<Vec<u8>> {
    loop {
        match inbound.message().await.ok()??.message {
            Some(ssh_tunnel_request::Message::Data(data)) => return Some(data),
            Some(ssh_tunnel_request::Message::Hostname(_)) => {
                tracing::warn!("ignoring hostname message on open tunnel");
            }
            None => {}
        }
    }
}

/// Copies SSH server bytes to the client until the server closes its end or the client goes
/// away.
async fn copy_downstream(
    mut reader: ReadHalf<DuplexStream>,
    tx: &mpsc::Sender<Result<SshTunnelResponse, Status>>,
) {
    let mut buffer = vec![0; TUNNEL_CHUNK_SIZE];

    while let Ok(count @ 1..) = reader.read(&mut buffer).await {
        let response = SshTunnelResponse {
            data: buffer[..count].to_vec(),
        };

        if tx.send(Ok(response)).await.is_err() {
            return;
        }
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
async fn update_anvil_sandboxes(change: SecretChange<'_>) -> Result<Vec<String>, Status> {
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

/// An interactive process in a sandbox, with its input and controls.
struct Session {
    handle: ExecHandle,
    stdin: ExecSink,
    control: ExecControl,
}

impl Session {
    /// Starts the command with a terminal in the sandbox.
    async fn open(sb: &Sandbox, command: String, args: Vec<String>) -> Result<Self, Status> {
        let mut handle = sb
            .exec_stream_with(command, |e| e.args(args).stdin_pipe().tty(true))
            .await
            .map_err(|_| Status::internal("failed to start session"))?;

        let stdin = handle
            .take_stdin()
            .ok_or_else(|| Status::internal("session has no stdin"))?;
        let control = handle.control();

        Ok(Self {
            handle,
            stdin,
            control,
        })
    }

    /// Sets the terminal size of the session, ending the session when that fails.
    async fn resize(&self, width: u16, height: u16) -> Result<(), Status> {
        if self.control.resize(height, width).await.is_err() {
            end_session(&self.control).await;
            return Err(Status::internal("failed to resize session"));
        }

        Ok(())
    }
}

/// Forwards client input to the session and session output to the client until either side ends.
async fn run_session(
    mut inbound: Streaming<AttachRequest>,
    mut session: Session,
    tx: mpsc::Sender<Result<AttachResponse, Status>>,
) {
    loop {
        let flow = tokio::select! {
            message = inbound.message() => {
                handle_client_message(message, &session.stdin, &session.control).await
            }
            event = session.handle.recv() => forward_event(event, &tx, &session.control).await,
            _ = tx.closed() => disconnect(&session.control).await,
        };

        if flow.is_break() {
            return;
        }
    }
}

/// Applies a message from the client to the session, ending the session when the client is gone.
async fn handle_client_message(
    message: Result<Option<AttachRequest>, Status>,
    stdin: &ExecSink,
    control: &ExecControl,
) -> ControlFlow<()> {
    match message {
        Ok(Some(AttachRequest {
            message: Some(message),
        })) => apply_client_message(message, stdin, control).await,
        Ok(Some(AttachRequest { message: None })) => {}
        Ok(None) | Err(_) => return disconnect(control).await,
    }

    ControlFlow::Continue(())
}

/// Writes client input to the session or resizes it.
async fn apply_client_message(
    message: attach_request::Message,
    stdin: &ExecSink,
    control: &ExecControl,
) {
    match message {
        attach_request::Message::Input(input) => write_input(stdin, input.data).await,
        attach_request::Message::Resize(size) => resize_session(control, size).await,
        attach_request::Message::Start(_) => {
            tracing::warn!("ignoring start message on running session");
        }
    }
}

/// Writes client input to the session, logging a warning when that fails.
async fn write_input(stdin: &ExecSink, data: Vec<u8>) {
    if let Err(err) = stdin.write(data).await {
        tracing::warn!("failed to write session input: {err}");
    }
}

/// Resizes the session terminal, logging a warning when that fails.
async fn resize_session(control: &ExecControl, size: AttachResize) {
    let Ok((width, height)) = window_size(Some(size)) else {
        tracing::warn!("ignoring invalid resize request");
        return;
    };

    if let Err(err) = control.resize(height, width).await {
        tracing::warn!("failed to resize session: {err}");
    }
}

/// Sends session output and its exit code to the client, stopping when the session ends.
async fn forward_event(
    event: Option<ExecEvent>,
    tx: &mpsc::Sender<Result<AttachResponse, Status>>,
    control: &ExecControl,
) -> ControlFlow<()> {
    match event {
        Some(ExecEvent::Stdout(data) | ExecEvent::Stderr(data)) => {
            forward_output(data.to_vec(), tx, control).await
        }
        Some(ExecEvent::Exited { code }) => {
            let response = AttachResponse {
                message: Some(attach_response::Message::ExitCode(code)),
            };
            let _ = tx.send(Ok(response)).await;
            ControlFlow::Break(())
        }
        Some(ExecEvent::Failed(failed)) => {
            tracing::warn!("session failed to start: {failed:?}");
            let _ = tx
                .send(Err(Status::internal("session failed to start")))
                .await;
            ControlFlow::Break(())
        }
        Some(_) => ControlFlow::Continue(()),
        None => ControlFlow::Break(()),
    }
}

/// Sends session output to the client, ending the session when the client is gone.
async fn forward_output(
    data: Vec<u8>,
    tx: &mpsc::Sender<Result<AttachResponse, Status>>,
    control: &ExecControl,
) -> ControlFlow<()> {
    let response = AttachResponse {
        message: Some(attach_response::Message::Output(data)),
    };

    if tx.send(Ok(response)).await.is_err() {
        end_session(control).await;
        return ControlFlow::Break(());
    }

    ControlFlow::Continue(())
}

/// Ends the session because the client disconnected.
async fn disconnect(control: &ExecControl) -> ControlFlow<()> {
    tracing::info!("client disconnected, ending session");
    end_session(control).await;
    ControlFlow::Break(())
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

/// Returns the requested image, or the default image when the request doesn't name one.
fn sandbox_image(image: &str) -> &str {
    if image.is_empty() {
        anvil_spec::DEFAULT_IMAGE
    } else {
        image
    }
}

/// Converts requested resources to vCPUs and MiB of memory, using the default resources when
/// the request has none.
fn sandbox_resources(resources: Option<SandboxResources>) -> Result<(u8, u32), Status> {
    let resources = resources.unwrap_or_else(|| {
        let defaults = SandboxResourcesSpec::default();

        SandboxResources {
            cpu: defaults.cpu.into(),
            memory: defaults.memory,
        }
    });

    let cpus = u8::try_from(resources.cpu)
        .ok()
        .filter(|cpus| *cpus > 0)
        .ok_or_else(|| Status::invalid_argument("cpu must be between 1 and 255"))?;
    let memory_mib = anvil_spec::parse_memory_mib(&resources.memory)
        .map_err(|err| Status::invalid_argument(err.to_string()))?;

    Ok((cpus, memory_mib))
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
    let incoming = UnixListenerStream::new(listener);

    Server::builder()
        .trace_fn(|req| tracing::info_span!("grpc", path = %req.uri().path()))
        .add_service(SandboxManagementServiceServer::new(AnvilServer::new(
            secrets,
        )))
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

    /// Returns a store for tests that never set a secret.
    fn unused_store() -> SecretStore {
        SecretStore::new(temp_path("unused-secrets.yml"))
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("anvil-test-{}-{}", std::process::id(), name))
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
        while tokio::net::UnixStream::connect(path).await.is_err() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
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
    fn sandbox_image_falls_back_to_default() {
        assert_eq!(sandbox_image("alpine:3.22"), "alpine:3.22");
        assert_eq!(sandbox_image(""), anvil_spec::DEFAULT_IMAGE);
    }

    #[test]
    fn sandbox_resources_converts_request() {
        let resources = SandboxResources {
            cpu: 4,
            memory: "8 GiB".to_string(),
        };

        assert_eq!(sandbox_resources(Some(resources)).unwrap(), (4, 8192));
    }

    #[test]
    fn sandbox_resources_falls_back_to_defaults() {
        assert_eq!(sandbox_resources(None).unwrap(), (2, 4096));
    }

    #[test]
    fn sandbox_resources_rejects_invalid_values() {
        let cases = [(0, "1 GiB"), (256, "1 GiB"), (2, "lots"), (2, "")];

        for (cpu, memory) in cases {
            let resources = SandboxResources {
                cpu,
                memory: memory.to_string(),
            };

            assert_eq!(
                sandbox_resources(Some(resources)).unwrap_err().code(),
                tonic::Code::InvalidArgument,
                "cpu {cpu}, memory {memory:?} should be rejected"
            );
        }
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
    async fn run_fails_when_socket_directory_does_not_exist() {
        let path = temp_path("missing-dir").join("anvil.sock");

        let result = run(&path, unused_store()).await;

        assert!(matches!(result, Err(ServerError::InvalidSocketPath(_))));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn set_secret_rejects_invalid_secret_without_storing_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.yml");
        let server = AnvilServer::new(SecretStore::new(&path));

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
        let server = AnvilServer::new(store);

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
        let server = AnvilServer::new(unused_store());

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
        let store = SecretStore::new(dir.path().join("secrets.yml"));
        let secret = Secret::new("A".into(), "value".into(), vec!["example.com".into()]).unwrap();
        store.set(secret).unwrap();
        let server = AnvilServer::new(store);

        let status = server
            .remove_secret(Request::new(RemoveSecretRequest {
                name: "B".to_string(),
            }))
            .await
            .expect_err("removing an unknown secret should fail");

        assert_eq!(status.code(), tonic::Code::NotFound);
        assert_eq!(server.secrets.load().unwrap().len(), 1);
    }
}

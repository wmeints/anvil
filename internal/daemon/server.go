package daemon

import (
	"context"
	"errors"
	"io"
	"math"

	containerd "github.com/containerd/containerd/v2/client"
	"github.com/wmeints/anvil/api/v1alpha1"
	"github.com/wmeints/anvil/internal/sandbox"
)

// ErrInvalidSessionStart is returned when a session doesn't begin with a start
// message.
var ErrInvalidSessionStart = errors.New("invalid session start message")

type anvilService struct {
	v1alpha1.UnimplementedAnvilServiceServer
	containerClient *containerd.Client
}

func (srv *anvilService) ListSandboxes(
	ctx context.Context, req *v1alpha1.ListSandboxesRequest,
) (resp *v1alpha1.ListSandboxesResponse, err error) {
	containers := srv.containerClient.ContainerService()

	result, err := containers.List(ctx, "labels.com.github.wmeints.anvil==true")
	if err != nil {
		return nil, err
	}

	var sandboxes []*v1alpha1.SandboxSummary

	for _, entry := range result {
		sandboxes = append(sandboxes, &v1alpha1.SandboxSummary{
			Name: entry.ID,
		})
	}

	return &v1alpha1.ListSandboxesResponse{
		Sandboxes: sandboxes,
	}, nil
}

// CreateSandbox creates a new sandbox and starts it in the background.
func (srv *anvilService) CreateSandbox(
	ctx context.Context, req *v1alpha1.CreateSandboxRequest,
) (resp *v1alpha1.CreateSandboxResponse, err error) {
	sb, err := sandbox.NewSandbox(req.Name, req.Image)
	if err != nil {
		return nil, err
	}

	err = sb.Start(ctx, srv.containerClient)
	if err != nil {
		return nil, err
	}

	return &v1alpha1.CreateSandboxResponse{}, nil
}

// RemoveSandbox stops the VM of an existing sandbox and removes it.
func (srv *anvilService) RemoveSandbox(
	ctx context.Context, req *v1alpha1.RemoveSandboxRequest,
) (resp *v1alpha1.RemoveSandboxResponse, err error) {
	if err := sandbox.Remove(ctx, srv.containerClient, req.Name); err != nil {
		return nil, err
	}

	return &v1alpha1.RemoveSandboxResponse{}, nil
}

// StopSandbox hibernates an existing sandbox: it stops its VM and keeps its
// disk.
func (srv *anvilService) StopSandbox(
	ctx context.Context, req *v1alpha1.StopSandboxRequest,
) (*v1alpha1.StopSandboxResponse, error) {
	timeout := req.GetTimeout().AsDuration()

	if err := sandbox.Stop(ctx, srv.containerClient, req.Name, timeout); err != nil {
		return nil, err
	}

	return &v1alpha1.StopSandboxResponse{}, nil
}

// StartSandbox boots the VM of an existing sandbox that isn't running.
func (srv *anvilService) StartSandbox(
	ctx context.Context, req *v1alpha1.StartSandboxRequest,
) (*v1alpha1.StartSandboxResponse, error) {
	if err := sandbox.Start(ctx, srv.containerClient, req.Name); err != nil {
		return nil, err
	}

	return &v1alpha1.StartSandboxResponse{}, nil
}

// AttachSandbox starts a session in a sandbox and streams its terminal I/O
// between the client and the session process until the process exits.
func (srv *anvilService) AttachSandbox(
	stream v1alpha1.AnvilService_AttachSandboxServer,
) error {
	ctx := stream.Context()

	req, err := stream.Recv()
	if err != nil {
		return err
	}

	start := req.GetStart()
	if start == nil {
		return ErrInvalidSessionStart
	}

	stdinReader, stdinWriter := io.Pipe()

	// Closing the reader unblocks pending input writes once the session ends.
	defer func() {
		_ = stdinReader.Close()
	}()

	output := &sessionWriter{stream: stream}

	session, err := sandbox.StartSession(ctx, srv.containerClient, start.Sandbox,
		sandbox.SessionOptions{
			Args:   start.Args,
			Width:  start.GetSize().GetWidth(),
			Height: start.GetSize().GetHeight(),
			Stdin:  stdinReader,
			Stdout: output,
		})
	if err != nil {
		return err
	}

	defer func() {
		_ = session.Close(context.WithoutCancel(ctx))
	}()

	go receiveSessionInput(stream, session, stdinWriter)

	exitCode, err := session.Wait()
	if err != nil {
		return err
	}

	return output.send(&v1alpha1.AttachSandboxResponse{
		Msg: &v1alpha1.AttachSandboxResponse_ExitCode{ExitCode: toExitCode(exitCode)},
	})
}

// toExitCode converts a process exit status to the exit code in the control
// API. Statuses beyond the int32 range can't come from a real process, so they
// saturate instead of wrapping around to a misleading value.
func toExitCode(status uint32) int32 {
	if status > math.MaxInt32 {
		return math.MaxInt32
	}

	return int32(status)
}

// receiveSessionInput forwards the input and resize messages from the client
// to the session until the client stops sending.
func receiveSessionInput(
	stream v1alpha1.AnvilService_AttachSandboxServer,
	session *sandbox.Session,
	stdin *io.PipeWriter,
) {
	for {
		req, err := stream.Recv()
		if err != nil {
			_ = stdin.CloseWithError(err)
			return
		}

		handleSessionRequest(stream.Context(), req, session, stdin)
	}
}

func handleSessionRequest(
	ctx context.Context,
	req *v1alpha1.AttachSandboxRequest,
	session *sandbox.Session,
	stdin io.Writer,
) {
	switch msg := req.Msg.(type) {
	case *v1alpha1.AttachSandboxRequest_Input:
		_, _ = stdin.Write(msg.Input.GetStdin())
	case *v1alpha1.AttachSandboxRequest_Resize:
		_ = session.Resize(ctx, msg.Resize.GetWidth(), msg.Resize.GetHeight())
	}
}

func newAnvilService(containerClient *containerd.Client) (*anvilService, error) {
	return &anvilService{
		containerClient: containerClient,
	}, nil
}

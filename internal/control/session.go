package control

import (
	"context"
	"errors"
	"io"
	"sync"

	"github.com/wmeints/anvil/api/v1alpha1"
)

// ErrSessionEnded is returned when the daemon closes the session without
// reporting the exit code of the session process.
var ErrSessionEnded = errors.New("session ended without an exit code")

// WindowSize is the size of the terminal attached to a session.
type WindowSize struct {
	Width  uint32
	Height uint32
}

// SessionOptions configures a session started through the daemon.
type SessionOptions struct {
	Sandbox string
	Args    []string
	Size    WindowSize
	Stdin   io.Reader
	Stdout  io.Writer
	// Resize receives new terminal sizes while the session runs. Optional.
	Resize <-chan WindowSize
}

// sessionSender serializes sends on the stream, gRPC forbids concurrent sends.
type sessionSender struct {
	mu     sync.Mutex
	stream v1alpha1.AnvilService_AttachSandboxClient
}

func (s *sessionSender) send(req *v1alpha1.AttachSandboxRequest) error {
	s.mu.Lock()
	defer s.mu.Unlock()

	return s.stream.Send(req)
}

// RunSession runs a command in a sandbox, connects the terminal streams to it
// and returns the exit code of the command once it exits.
func (client *Client) RunSession(
	ctx context.Context, opts SessionOptions,
) (int, error) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	stream, err := client.anvilClient.AttachSandbox(ctx)
	if err != nil {
		return 0, err
	}

	sender := &sessionSender{stream: stream}

	err = sender.send(&v1alpha1.AttachSandboxRequest{
		Msg: &v1alpha1.AttachSandboxRequest_Start{Start: &v1alpha1.AttachStart{
			Sandbox: opts.Sandbox,
			Args:    opts.Args,
			Size:    windowSize(opts.Size),
		}},
	})
	if err != nil {
		return 0, err
	}

	go sendInput(sender, opts.Stdin)
	go sendResizes(ctx, sender, opts.Resize)

	return receiveOutput(stream, opts.Stdout)
}

func windowSize(size WindowSize) *v1alpha1.WindowSize {
	return &v1alpha1.WindowSize{Width: size.Width, Height: size.Height}
}

// Write sends the data as terminal input to the session.
func (s *sessionSender) Write(p []byte) (int, error) {
	input := &v1alpha1.SessionInput{Stdin: append([]byte(nil), p...)}
	msg := &v1alpha1.AttachSandboxRequest_Input{Input: input}

	if err := s.send(&v1alpha1.AttachSandboxRequest{Msg: msg}); err != nil {
		return 0, err
	}

	return len(p), nil
}

func sendInput(sender *sessionSender, stdin io.Reader) {
	_, _ = io.Copy(sender, stdin)
}

func sendResizes(ctx context.Context, sender *sessionSender, resize <-chan WindowSize) {
	for {
		select {
		case <-ctx.Done():
			return
		case size := <-resize:
			msg := &v1alpha1.AttachSandboxRequest_Resize{Resize: windowSize(size)}
			_ = sender.send(&v1alpha1.AttachSandboxRequest{Msg: msg})
		}
	}
}

func receiveOutput(
	stream v1alpha1.AnvilService_AttachSandboxClient, stdout io.Writer,
) (int, error) {
	for {
		resp, err := stream.Recv()
		if err != nil {
			return 0, sessionError(err)
		}

		if exitCode, ok := resp.Msg.(*v1alpha1.AttachSandboxResponse_ExitCode); ok {
			return int(exitCode.ExitCode), nil
		}

		_, _ = stdout.Write(resp.GetStdout())
	}
}

func sessionError(err error) error {
	if errors.Is(err, io.EOF) {
		return ErrSessionEnded
	}

	return err
}

package control

import (
	"bytes"
	"context"
	"errors"
	"net"
	"slices"
	"strings"
	"testing"

	"github.com/wmeints/anvil/api/v1alpha1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/test/bufconn"
)

// echoService echoes the first input back and exits with the number of args.
type echoService struct {
	v1alpha1.UnimplementedAnvilServiceServer
	start *v1alpha1.AttachStart
}

// echoExitCode is the exit code echoService reports when the session ends.
const echoExitCode = 3

func (srv *echoService) AttachSandbox(
	stream v1alpha1.AnvilService_AttachSandboxServer,
) error {
	req, err := stream.Recv()
	if err != nil {
		return err
	}

	srv.start = req.GetStart()

	req, err = stream.Recv()
	if err != nil {
		return err
	}

	err = stream.Send(&v1alpha1.AttachSandboxResponse{
		Msg: &v1alpha1.AttachSandboxResponse_Stdout{Stdout: req.GetInput().GetStdin()},
	})
	if err != nil {
		return err
	}

	return stream.Send(&v1alpha1.AttachSandboxResponse{
		Msg: &v1alpha1.AttachSandboxResponse_ExitCode{
			ExitCode: echoExitCode,
		},
	})
}

func newTestClient(t *testing.T, srv v1alpha1.AnvilServiceServer) *Client {
	t.Helper()

	listener := bufconn.Listen(1024 * 1024)
	server := grpc.NewServer()
	v1alpha1.RegisterAnvilServiceServer(server, srv)

	go func() {
		_ = server.Serve(listener)
	}()

	t.Cleanup(server.Stop)

	dialer := func(ctx context.Context, _ string) (net.Conn, error) {
		return listener.DialContext(ctx)
	}

	conn, err := grpc.NewClient("passthrough:///bufnet",
		grpc.WithContextDialer(dialer),
		grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		t.Fatal(err)
	}

	t.Cleanup(func() { _ = conn.Close() })

	return &Client{anvilClient: v1alpha1.NewAnvilServiceClient(conn), serverConn: conn}
}

func TestRunSession(t *testing.T) {
	srv := &echoService{}
	client := newTestClient(t, srv)

	var stdout bytes.Buffer

	code, err := client.RunSession(context.Background(), SessionOptions{
		Sandbox: "demo",
		Args:    []string{"sh", "-c", "exit"},
		Size:    WindowSize{Width: 80, Height: 24},
		Stdin:   strings.NewReader("hello"),
		Stdout:  &stdout,
	})
	if err != nil {
		t.Fatal(err)
	}

	if code != echoExitCode {
		t.Errorf("expected exit code %d, got %d", echoExitCode, code)
	}

	if want := []string{"sh", "-c", "exit"}; !slices.Equal(srv.start.Args, want) {
		t.Errorf("expected args %v, got %v", want, srv.start.Args)
	}

	if stdout.String() != "hello" {
		t.Errorf("expected output %q, got %q", "hello", stdout.String())
	}

	assertStart(t, srv.start)
}

func assertStart(t *testing.T, start *v1alpha1.AttachStart) {
	t.Helper()

	if start.Sandbox != "demo" {
		t.Errorf("unexpected start message: %v", start)
	}

	if start.Size.Width != 80 || start.Size.Height != 24 {
		t.Errorf("unexpected window size: %v", start.Size)
	}
}

// closingService ends the session without reporting an exit code.
type closingService struct {
	v1alpha1.UnimplementedAnvilServiceServer
}

func (srv *closingService) AttachSandbox(
	stream v1alpha1.AnvilService_AttachSandboxServer,
) error {
	_, err := stream.Recv()
	return err
}

func TestRunSessionWithoutExitCode(t *testing.T) {
	client := newTestClient(t, &closingService{})

	_, err := client.RunSession(context.Background(), SessionOptions{
		Sandbox: "demo",
		Args:    []string{"sh"},
		Stdin:   strings.NewReader(""),
		Stdout:  &bytes.Buffer{},
	})
	if !errors.Is(err, ErrSessionEnded) {
		t.Errorf("expected ErrSessionEnded, got %v", err)
	}
}

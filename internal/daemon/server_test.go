package daemon

import (
	"context"
	"net"
	"strings"
	"testing"

	containerd "github.com/containerd/containerd/v2/client"
	"github.com/wmeints/anvil/api/v1alpha1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/test/bufconn"
)

// newTestServiceClient serves the anvil service over an in-memory connection
// and returns a client connected to it.
func newTestServiceClient(
	t *testing.T, cc *containerd.Client,
) v1alpha1.AnvilServiceClient {
	t.Helper()

	lis := bufconn.Listen(1024 * 1024)

	svc, err := newAnvilService(cc)
	if err != nil {
		t.Fatal(err)
	}

	srv := grpc.NewServer()
	v1alpha1.RegisterAnvilServiceServer(srv, svc)

	go func() { _ = srv.Serve(lis) }()

	t.Cleanup(srv.Stop)

	conn, err := grpc.NewClient("passthrough:///bufnet",
		grpc.WithContextDialer(func(ctx context.Context, _ string) (net.Conn, error) {
			return lis.DialContext(ctx)
		}),
		grpc.WithTransportCredentials(insecure.NewCredentials()),
	)
	if err != nil {
		t.Fatal(err)
	}

	t.Cleanup(func() { _ = conn.Close() })

	return v1alpha1.NewAnvilServiceClient(conn)
}

func TestCreateSandboxRejectsInvalidName(t *testing.T) {
	client := newTestServiceClient(t, nil)

	_, err := client.CreateSandbox(t.Context(), &v1alpha1.CreateSandboxRequest{
		Name:  "Invalid Name",
		Image: "ubuntu:26.04",
	})
	if err == nil {
		t.Fatal("expected an error for an invalid sandbox name")
	}
}

func TestCreateSandboxRejectsMissingImage(t *testing.T) {
	client := newTestServiceClient(t, nil)

	_, err := client.CreateSandbox(t.Context(), &v1alpha1.CreateSandboxRequest{
		Name: "sandbox",
	})
	if err == nil {
		t.Fatal("expected an error for a missing image")
	}
}

func TestAttachSandboxRequiresStartMessage(t *testing.T) {
	client := newTestServiceClient(t, nil)

	stream, err := client.AttachSandbox(t.Context())
	if err != nil {
		t.Fatal(err)
	}

	err = stream.Send(&v1alpha1.AttachSandboxRequest{
		Msg: &v1alpha1.AttachSandboxRequest_Input{
			Input: &v1alpha1.SessionInput{Stdin: []byte("ls\n")},
		},
	})
	if err != nil {
		t.Fatal(err)
	}

	_, err = stream.Recv()
	if err == nil || !strings.Contains(err.Error(), ErrInvalidSessionStart.Error()) {
		t.Fatalf("Recv() error = %v, want %v", err, ErrInvalidSessionStart)
	}
}

func TestAttachSandboxEndsWhenClientSendsNothing(t *testing.T) {
	client := newTestServiceClient(t, nil)

	stream, err := client.AttachSandbox(t.Context())
	if err != nil {
		t.Fatal(err)
	}

	if err := stream.CloseSend(); err != nil {
		t.Fatal(err)
	}

	if _, err := stream.Recv(); err == nil {
		t.Fatal("expected the session to end without a start message")
	}
}

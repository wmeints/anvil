//go:build integration

package daemon

import (
	"cmp"
	"context"
	"errors"
	"io"
	"os"
	"slices"
	"strings"
	"testing"
	"time"

	containerd "github.com/containerd/containerd/v2/client"
	"github.com/wmeints/anvil/api/v1alpha1"
	"github.com/wmeints/anvil/internal/paths"
	"github.com/wmeints/anvil/internal/sandbox"
	"google.golang.org/protobuf/types/known/durationpb"
)

// testImage is the image of the sandboxes in the integration tests. Set
// ANVIL_TEST_IMAGE to test an image that isn't published yet, for example one
// pushed to a local registry.
var testImage = cmp.Or(
	os.Getenv("ANVIL_TEST_IMAGE"), "ghcr.io/wmeints/anvil-base:latest")

func newTestContainerClient(t *testing.T) *containerd.Client {
	t.Helper()

	cc, err := containerd.New(
		paths.ContainerRuntimeSocket(),
		containerd.WithDefaultNamespace("anvil-test"),
	)
	if err != nil {
		t.Fatal(err)
	}

	t.Cleanup(func() { _ = cc.Close() })

	return cc
}

// createTestSandbox creates a sandbox through the service and removes it again
// when the test finishes.
func createTestSandbox(
	ctx context.Context, t *testing.T, cc *containerd.Client,
	client v1alpha1.AnvilServiceClient, name string,
) {
	t.Helper()

	_ = sandbox.Remove(context.Background(), cc, name)
	t.Cleanup(func() { _ = sandbox.Remove(context.Background(), cc, name) })

	_, err := client.CreateSandbox(ctx, &v1alpha1.CreateSandboxRequest{
		Name:  name,
		Image: testImage,
	})
	if err != nil {
		t.Fatal(err)
	}
}

func listSandboxNames(
	ctx context.Context, t *testing.T, client v1alpha1.AnvilServiceClient,
) []string {
	t.Helper()

	resp, err := client.ListSandboxes(ctx, &v1alpha1.ListSandboxesRequest{})
	if err != nil {
		t.Fatal(err)
	}

	var names []string
	for _, sb := range resp.GetSandboxes() {
		names = append(names, sb.GetName())
	}

	return names
}

// attach starts a session in the sandbox and sends the given requests after
// the start message.
func attach(
	ctx context.Context, t *testing.T, client v1alpha1.AnvilServiceClient,
	start *v1alpha1.AttachStart, reqs ...*v1alpha1.AttachSandboxRequest,
) v1alpha1.AnvilService_AttachSandboxClient {
	t.Helper()

	stream, err := client.AttachSandbox(ctx)
	if err != nil {
		t.Fatal(err)
	}

	reqs = append([]*v1alpha1.AttachSandboxRequest{
		{Msg: &v1alpha1.AttachSandboxRequest_Start{Start: start}},
	}, reqs...)

	for _, req := range reqs {
		if err := stream.Send(req); err != nil {
			t.Fatal(err)
		}
	}

	return stream
}

// collectSession reads the output of a session until it reports its exit code.
func collectSession(
	t *testing.T, stream v1alpha1.AnvilService_AttachSandboxClient,
) (string, int32) {
	t.Helper()

	var output strings.Builder

	for {
		resp, err := stream.Recv()
		if err != nil {
			t.Fatalf("session ended without an exit code: %v", err)
		}

		switch msg := resp.Msg.(type) {
		case *v1alpha1.AttachSandboxResponse_Stdout:
			output.Write(msg.Stdout)
		case *v1alpha1.AttachSandboxResponse_ExitCode:
			return output.String(), msg.ExitCode
		}
	}
}

func TestSandboxLifecycle(t *testing.T) {
	cc := newTestContainerClient(t)
	client := newTestServiceClient(t, cc)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "server-lifecycle-test"
	createTestSandbox(ctx, t, cc, client, name)

	if names := listSandboxNames(ctx, t, client); !slices.Contains(names, name) {
		t.Fatalf("ListSandboxes() = %v, want it to contain %q", names, name)
	}

	_, err := client.RemoveSandbox(ctx, &v1alpha1.RemoveSandboxRequest{Name: name})
	if err != nil {
		t.Fatal(err)
	}

	if names := listSandboxNames(ctx, t, client); slices.Contains(names, name) {
		t.Fatalf("ListSandboxes() = %v, want it to not contain %q", names, name)
	}
}

func TestStopSandboxEndsSessionsAndKeepsFiles(t *testing.T) {
	cc := newTestContainerClient(t)
	client := newTestServiceClient(t, cc)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "server-stop-test"
	createTestSandbox(ctx, t, cc, client, name)

	stream := attach(ctx, t, client, &v1alpha1.AttachStart{
		Sandbox: name,
		Args: []string{
			"sh", "-c", "echo hi > /home/agent/marker; echo ready; sleep 600",
		},
	})

	waitForOutput(t, stream, "ready")

	_, err := client.StopSandbox(ctx, &v1alpha1.StopSandboxRequest{
		Name:    name,
		Timeout: durationpb.New(time.Minute),
	})
	if err != nil {
		t.Fatal(err)
	}

	if _, code := collectSession(t, stream); code == 0 {
		t.Error("expected the attached session to end with a non-zero exit code")
	}

	output, _ := collectSession(t, attach(ctx, t, client, &v1alpha1.AttachStart{
		Sandbox: name,
		Args:    []string{"cat", "/home/agent/marker"},
	}))

	if !strings.Contains(output, "hi") {
		t.Errorf("output = %q, want the marker file to survive the stop", output)
	}
}

// waitForOutput reads the output of a session until it contains want.
func waitForOutput(
	t *testing.T, stream v1alpha1.AnvilService_AttachSandboxClient, want string,
) {
	t.Helper()

	var output strings.Builder

	for !strings.Contains(output.String(), want) {
		resp, err := stream.Recv()
		if err != nil {
			t.Fatalf("session ended before printing %q: %v", want, err)
		}

		output.Write(resp.GetStdout())
	}
}

func TestStopSandboxMissing(t *testing.T) {
	client := newTestServiceClient(t, newTestContainerClient(t))

	_, err := client.StopSandbox(t.Context(), &v1alpha1.StopSandboxRequest{
		Name:    "does-not-exist",
		Timeout: durationpb.New(time.Second),
	})

	want := sandbox.ErrSandboxNotFound.Error()
	if err == nil || !strings.Contains(err.Error(), want) {
		t.Fatalf("StopSandbox() error = %v, want %v", err, sandbox.ErrSandboxNotFound)
	}
}

func TestStartSandboxBootsStoppedSandbox(t *testing.T) {
	cc := newTestContainerClient(t)
	client := newTestServiceClient(t, cc)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "server-start-test"
	createTestSandbox(ctx, t, cc, client, name)

	_, err := client.StopSandbox(ctx, &v1alpha1.StopSandboxRequest{
		Name:    name,
		Timeout: durationpb.New(time.Minute),
	})
	if err != nil {
		t.Fatal(err)
	}

	_, err = client.StartSandbox(ctx, &v1alpha1.StartSandboxRequest{Name: name})
	if err != nil {
		t.Fatal(err)
	}

	assertRunning(ctx, t, cc, name)
}

// assertRunning verifies that the sandbox has a running task.
func assertRunning(
	ctx context.Context, t *testing.T, cc *containerd.Client, name string,
) {
	t.Helper()

	container, err := cc.LoadContainer(ctx, name)
	if err != nil {
		t.Fatal(err)
	}

	task, err := container.Task(ctx, nil)
	if err != nil {
		t.Fatalf("expected the sandbox to have a task, got %v", err)
	}

	status, err := task.Status(ctx)
	if err != nil || status.Status != containerd.Running {
		t.Errorf("task status = %v (%v), want %s", status.Status, err,
			containerd.Running)
	}
}

func TestStartSandboxMissing(t *testing.T) {
	client := newTestServiceClient(t, newTestContainerClient(t))

	_, err := client.StartSandbox(t.Context(),
		&v1alpha1.StartSandboxRequest{Name: "does-not-exist"})

	want := sandbox.ErrSandboxNotFound.Error()
	if err == nil || !strings.Contains(err.Error(), want) {
		t.Fatalf("StartSandbox() error = %v, want %v", err, sandbox.ErrSandboxNotFound)
	}
}

func TestRemoveSandboxMissing(t *testing.T) {
	client := newTestServiceClient(t, newTestContainerClient(t))

	_, err := client.RemoveSandbox(t.Context(),
		&v1alpha1.RemoveSandboxRequest{Name: "does-not-exist"})
	if err == nil {
		t.Fatal("expected an error when removing a missing sandbox")
	}
}

func TestAttachSandboxStreamsOutputAndExitCode(t *testing.T) {
	cc := newTestContainerClient(t)
	client := newTestServiceClient(t, cc)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "server-attach-test"
	createTestSandbox(ctx, t, cc, client, name)

	stream := attach(ctx, t, client, &v1alpha1.AttachStart{
		Sandbox: name,
		Args:    []string{"sh", "-c", "echo hello; exit 3"},
	})

	output, code := collectSession(t, stream)

	if code != 3 {
		t.Errorf("exit code = %d, want 3", code)
	}

	if !strings.Contains(output, "hello") {
		t.Errorf("output = %q, want it to contain %q", output, "hello")
	}

	if _, err := stream.Recv(); !errors.Is(err, io.EOF) {
		t.Errorf("expected the stream to end after the exit code, got %v", err)
	}
}

func TestAttachSandboxForwardsInputAndResize(t *testing.T) {
	cc := newTestContainerClient(t)
	client := newTestServiceClient(t, cc)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "server-input-test"
	createTestSandbox(ctx, t, cc, client, name)

	// The input is sent after the resize, so the size is changed by the time
	// the process reads its input.
	stream := attach(ctx, t, client,
		&v1alpha1.AttachStart{
			Sandbox: name,
			Args:    []string{"sh", "-c", "read line; echo got-$line; stty size"},
			Size:    &v1alpha1.WindowSize{Width: 80, Height: 24},
		},
		&v1alpha1.AttachSandboxRequest{
			Msg: &v1alpha1.AttachSandboxRequest_Resize{
				Resize: &v1alpha1.WindowSize{Width: 132, Height: 43},
			},
		},
		&v1alpha1.AttachSandboxRequest{
			Msg: &v1alpha1.AttachSandboxRequest_Input{
				Input: &v1alpha1.SessionInput{Stdin: []byte("ping\n")},
			},
		},
	)

	output, code := collectSession(t, stream)

	if code != 0 {
		t.Errorf("exit code = %d, want 0", code)
	}

	for _, want := range []string{"got-ping", "43 132"} {
		if !strings.Contains(output, want) {
			t.Errorf("output = %q, want it to contain %q", output, want)
		}
	}
}

func TestAttachSandboxMissing(t *testing.T) {
	client := newTestServiceClient(t, newTestContainerClient(t))

	stream := attach(t.Context(), t, client, &v1alpha1.AttachStart{
		Sandbox: "does-not-exist",
		Args:    []string{"sh"},
	})

	_, err := stream.Recv()

	want := sandbox.ErrSandboxNotFound.Error()
	if err == nil || !strings.Contains(err.Error(), want) {
		t.Fatalf("Recv() error = %v, want %v", err, sandbox.ErrSandboxNotFound)
	}
}

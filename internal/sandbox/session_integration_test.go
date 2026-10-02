//go:build integration

package sandbox

import (
	"bytes"
	"cmp"
	"context"
	"errors"
	"io"
	"os"
	"strings"
	"syscall"
	"testing"
	"time"

	containerd "github.com/containerd/containerd/v2/client"
	"github.com/containerd/errdefs"
	"github.com/wmeints/anvil/internal/paths"
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

func removeSandbox(cc *containerd.Client, name string) {
	_ = Remove(context.Background(), cc, name)
}

func TestStartSession(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "session-test"
	startTestSandbox(ctx, t, cc, name)

	var stdout bytes.Buffer

	session, err := StartSession(ctx, cc, name, SessionOptions{
		Args:   []string{"sh", "-c", "echo hi $TERM; exit 3"},
		Width:  80,
		Height: 24,
		Stdin:  strings.NewReader(""),
		Stdout: &stdout,
	})
	if err != nil {
		t.Fatal(err)
	}

	defer func() { _ = session.Close(context.Background()) }()

	code, err := session.Wait()
	if err != nil {
		t.Fatal(err)
	}

	if code != 3 {
		t.Errorf("expected exit code 3, got %d", code)
	}

	if !strings.Contains(stdout.String(), "hi xterm-256color") {
		t.Errorf("expected output to contain %q, got %q",
			"hi xterm-256color", stdout.String())
	}
}

func TestStartSessionMissingSandbox(t *testing.T) {
	cc := newTestContainerClient(t)

	_, err := StartSession(context.Background(), cc, "does-not-exist",
		SessionOptions{Args: []string{"sh"}})
	if !errors.Is(err, ErrSandboxNotFound) {
		t.Errorf("expected ErrSandboxNotFound, got %v", err)
	}
}

func TestStartSessionUnknownCommand(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "session-unknown-command-test"
	startTestSandbox(ctx, t, cc, name)

	_, err := StartSession(ctx, cc, name, SessionOptions{
		Args:   []string{"/does-not-exist"},
		Stdin:  strings.NewReader(""),
		Stdout: io.Discard,
	})
	if !errors.Is(err, ErrSessionStartFailed) {
		t.Errorf("expected ErrSessionStartFailed, got %v", err)
	}
}

// assertSessionRuns starts a session in the sandbox and verifies it runs.
func assertSessionRuns(
	ctx context.Context, t *testing.T, cc *containerd.Client, name string,
) {
	t.Helper()

	out := runSession(ctx, t, cc, name, "echo started")
	if !strings.Contains(out, "started") {
		t.Errorf("expected output to contain %q, got %q", "started", out)
	}
}

// runSession runs a shell script in the sandbox and returns its output.
func runSession(
	ctx context.Context, t *testing.T, cc *containerd.Client, name, script string,
) string {
	t.Helper()

	out, _ := runSessionCode(ctx, t, cc, name, script)

	return out
}

// runSessionCode runs a shell script in the sandbox and returns its output and
// exit code.
func runSessionCode(
	ctx context.Context, t *testing.T, cc *containerd.Client, name, script string,
) (string, uint32) {
	t.Helper()

	var stdout bytes.Buffer

	session, err := StartSession(ctx, cc, name, SessionOptions{
		Args:   []string{"sh", "-c", script},
		Stdin:  strings.NewReader(""),
		Stdout: &stdout,
	})
	if err != nil {
		t.Fatal(err)
	}

	defer func() { _ = session.Close(context.Background()) }()

	code, err := session.Wait()
	if err != nil {
		t.Fatal(err)
	}

	return stdout.String(), code
}

func TestStartSessionStartsStoppedSandbox(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "session-stopped-test"
	container := startTestSandbox(ctx, t, cc, name)

	task, err := container.Task(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}

	exitC, err := task.Wait(ctx)
	if err != nil {
		t.Fatal(err)
	}

	if err := task.Kill(ctx, syscall.SIGKILL); err != nil {
		t.Fatal(err)
	}

	<-exitC

	assertSessionRuns(ctx, t, cc, name)
}

func TestStartSessionStartsSandboxWithoutTask(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "session-no-task-test"
	container := startTestSandbox(ctx, t, cc, name)

	task, err := container.Task(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}

	if _, err := task.Delete(ctx, containerd.WithProcessKill); err != nil {
		t.Fatal(err)
	}

	assertSessionRuns(ctx, t, cc, name)
}

func TestSessionRunsAsImageUser(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "session-user-test"
	startTestSandbox(ctx, t, cc, name)

	if out := runSession(ctx, t, cc, name, "id -u"); strings.TrimSpace(out) != "1000" {
		t.Errorf("id -u = %q, want 1000", out)
	}

	if out, code := runSessionCode(ctx, t, cc, name, "sudo -n true"); code != 0 {
		t.Errorf("sudo -n true exited with %d: %q", code, out)
	}
}

func TestSessionRunsDocker(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "session-docker-test"
	startTestSandbox(ctx, t, cc, name)

	// The entrypoint starts dockerd in the background while the sandbox boots.
	script := `timeout 60 sh -c 'until docker info >/dev/null 2>&1; do sleep 1; done'
		docker run --rm hello-world`

	out, code := runSessionCode(ctx, t, cc, name, script)
	if code != 0 || !strings.Contains(out, "Hello from Docker!") {
		t.Errorf("docker run exited with %d: %q", code, out)
	}
}

func TestStartRejectsImageWithoutUser(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "no-user-test"
	removeSandbox(cc, name)
	t.Cleanup(func() { removeSandbox(cc, name) })

	sb, err := NewSandbox(name, "ubuntu:26.04")
	if err != nil {
		t.Fatal(err)
	}

	if err := sb.Start(ctx, cc); !errors.Is(err, ErrInvalidImageUser) {
		t.Fatalf("error = %v, want %v", err, ErrInvalidImageUser)
	}

	if _, err := cc.LoadContainer(ctx, name); !errdefs.IsNotFound(err) {
		t.Errorf("expected no sandbox container, got %v", err)
	}
}

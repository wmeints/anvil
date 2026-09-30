//go:build integration

package sandbox

import (
	"bytes"
	"context"
	"errors"
	"strings"
	"syscall"
	"testing"
	"time"

	containerd "github.com/containerd/containerd/v2/client"
	"github.com/wmeints/anvil/internal/utils"
)

func newTestContainerClient(t *testing.T) *containerd.Client {
	t.Helper()

	cc, err := containerd.New(
		utils.ContainerRuntimeSocketPath(),
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

// assertSessionRuns starts a session in the sandbox and verifies it runs.
func assertSessionRuns(
	ctx context.Context, t *testing.T, cc *containerd.Client, name string,
) {
	t.Helper()

	var stdout bytes.Buffer

	session, err := StartSession(ctx, cc, name, SessionOptions{
		Args:   []string{"sh", "-c", "echo started"},
		Stdin:  strings.NewReader(""),
		Stdout: &stdout,
	})
	if err != nil {
		t.Fatal(err)
	}

	defer func() { _ = session.Close(context.Background()) }()

	if _, err := session.Wait(); err != nil {
		t.Fatal(err)
	}

	if !strings.Contains(stdout.String(), "started") {
		t.Errorf("expected output to contain %q, got %q", "started", stdout.String())
	}
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

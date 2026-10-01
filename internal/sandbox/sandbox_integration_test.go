//go:build integration

package sandbox

import (
	"context"
	"errors"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/containerd/containerd/api/services/tasks/v1"
	containerd "github.com/containerd/containerd/v2/client"
	"github.com/containerd/errdefs"
)

// startTestSandbox starts a sandbox with the given name and removes it again
// when the test finishes.
func startTestSandbox(
	ctx context.Context, t *testing.T, cc *containerd.Client, name string,
) containerd.Container {
	t.Helper()

	removeSandbox(cc, name)
	t.Cleanup(func() { removeSandbox(cc, name) })

	sb, err := NewSandbox(name, "ubuntu:26.04")
	if err != nil {
		t.Fatal(err)
	}

	if err := sb.Start(ctx, cc); err != nil {
		t.Fatal(err)
	}

	container, err := cc.LoadContainer(ctx, name)
	if err != nil {
		t.Fatal(err)
	}

	return container
}

// assertRemoved verifies that the task, the container and the snapshot of the
// sandbox no longer exist.
func assertRemoved(
	ctx context.Context, t *testing.T, cc *containerd.Client,
	name, snapshotter, snapshotKey string,
) {
	t.Helper()

	assertTaskRemoved(ctx, t, cc, name)

	if _, err := cc.LoadContainer(ctx, name); !errdefs.IsNotFound(err) {
		t.Errorf("expected the sandbox container to be gone, got %v", err)
	}

	_, err := cc.SnapshotService(snapshotter).Stat(ctx, snapshotKey)
	if !errdefs.IsNotFound(err) {
		t.Errorf("expected the sandbox snapshot to be gone, got %v", err)
	}
}

// assertTaskRemoved verifies that the task of the sandbox no longer exists. A
// task outlives the removal of its container, so it's looked up in the task list
// instead of through the container.
func assertTaskRemoved(
	ctx context.Context, t *testing.T, cc *containerd.Client, name string,
) {
	t.Helper()

	resp, err := cc.TaskService().List(ctx, &tasks.ListTasksRequest{})
	if err != nil {
		t.Fatal(err)
	}

	for _, task := range resp.Tasks {
		if task.ID == name {
			t.Errorf("expected the sandbox task to be gone, found it %s", task.Status)
		}
	}
}

func TestRemoveStopsSandbox(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "remove-test"
	container := startTestSandbox(ctx, t, cc, name)

	info, err := container.Info(ctx)
	if err != nil {
		t.Fatal(err)
	}

	if err := Remove(ctx, cc, name); err != nil {
		t.Fatal(err)
	}

	assertRemoved(ctx, t, cc, name, info.Snapshotter, info.SnapshotKey)
}

func TestRemoveStoppedSandbox(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "remove-stopped-test"
	container := startTestSandbox(ctx, t, cc, name)

	info, err := container.Info(ctx)
	if err != nil {
		t.Fatal(err)
	}

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

	if err := Remove(ctx, cc, name); err != nil {
		t.Fatal(err)
	}

	assertRemoved(ctx, t, cc, name, info.Snapshotter, info.SnapshotKey)
}

func TestRemoveSandboxWithoutTask(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "remove-no-task-test"
	container := startTestSandbox(ctx, t, cc, name)

	info, err := container.Info(ctx)
	if err != nil {
		t.Fatal(err)
	}

	task, err := container.Task(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}

	if _, err := task.Delete(ctx, containerd.WithProcessKill); err != nil {
		t.Fatal(err)
	}

	if err := Remove(ctx, cc, name); err != nil {
		t.Fatal(err)
	}

	assertRemoved(ctx, t, cc, name, info.Snapshotter, info.SnapshotKey)
}

func TestRemoveUnknownSandbox(t *testing.T) {
	cc := newTestContainerClient(t)

	err := Remove(context.Background(), cc, "does-not-exist")
	if !errors.Is(err, ErrSandboxNotFound) {
		t.Errorf("expected ErrSandboxNotFound, got %v", err)
	}
}

// assertStopped verifies that the VM of the sandbox stopped while its container
// and snapshot are kept.
func assertStopped(
	ctx context.Context, t *testing.T, cc *containerd.Client, name string,
) {
	t.Helper()

	assertTaskRemoved(ctx, t, cc, name)

	container, err := cc.LoadContainer(ctx, name)
	if err != nil {
		t.Fatalf("expected the sandbox container to exist, got %v", err)
	}

	info, err := container.Info(ctx)
	if err != nil {
		t.Fatal(err)
	}

	_, err = cc.SnapshotService(info.Snapshotter).Stat(ctx, info.SnapshotKey)
	if err != nil {
		t.Errorf("expected the sandbox snapshot to exist, got %v", err)
	}
}

// stopTimeout is long enough that a test only passes within it when the
// sandbox stops on SIGTERM.
const stopTimeout = time.Minute

func TestStopRunningSandbox(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "stop-test"
	startTestSandbox(ctx, t, cc, name)

	started := time.Now()

	if err := Stop(ctx, cc, name, stopTimeout); err != nil {
		t.Fatal(err)
	}

	if elapsed := time.Since(started); elapsed >= stopTimeout {
		t.Errorf("expected the sandbox to stop on SIGTERM, took %v", elapsed)
	}

	assertStopped(ctx, t, cc, name)
}

func TestStopKeepsFiles(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "stop-files-test"
	startTestSandbox(ctx, t, cc, name)

	runSession(ctx, t, cc, name, "echo hi > /root/marker")

	if err := Stop(ctx, cc, name, stopTimeout); err != nil {
		t.Fatal(err)
	}

	out := runSession(ctx, t, cc, name, "cat /root/marker")
	if !strings.Contains(out, "hi") {
		t.Errorf("expected the marker file to survive the stop, got %q", out)
	}
}

func TestStopStoppedSandbox(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "stop-stopped-test"
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

	if err := Stop(ctx, cc, name, stopTimeout); err != nil {
		t.Fatal(err)
	}

	assertStopped(ctx, t, cc, name)

	if err := Stop(ctx, cc, name, stopTimeout); err != nil {
		t.Fatalf("expected stopping a stopped sandbox again to succeed, got %v", err)
	}
}

func TestStopSandboxWithoutTask(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "stop-no-task-test"
	container := startTestSandbox(ctx, t, cc, name)

	task, err := container.Task(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}

	if _, err := task.Delete(ctx, containerd.WithProcessKill); err != nil {
		t.Fatal(err)
	}

	if err := Stop(ctx, cc, name, stopTimeout); err != nil {
		t.Fatal(err)
	}

	assertStopped(ctx, t, cc, name)
}

func TestStopUnknownSandbox(t *testing.T) {
	cc := newTestContainerClient(t)

	err := Stop(context.Background(), cc, "does-not-exist", stopTimeout)
	if !errors.Is(err, ErrSandboxNotFound) {
		t.Errorf("expected ErrSandboxNotFound, got %v", err)
	}
}

func TestStopKillsSandboxAfterTimeout(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "stop-timeout-test"
	removeSandbox(cc, name)
	t.Cleanup(func() { removeSandbox(cc, name) })

	sb, err := NewSandbox(name, "ubuntu:26.04")
	if err != nil {
		t.Fatal(err)
	}

	// Sandboxes from before anvil stop existed run an init process that
	// ignores SIGTERM.
	if err := sb.start(ctx, cc, []string{"sleep", "infinity"}); err != nil {
		t.Fatal(err)
	}

	timeout := 2 * time.Second
	started := time.Now()

	if err := Stop(ctx, cc, name, timeout); err != nil {
		t.Fatal(err)
	}

	if elapsed := time.Since(started); elapsed < timeout {
		t.Errorf("expected stop to wait for the timeout, took %v", elapsed)
	}

	assertStopped(ctx, t, cc, name)
}

func TestSandboxSurvivesKillingInitChild(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "init-child-test"
	container := startTestSandbox(ctx, t, cc, name)

	task, err := container.Task(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}

	exitC, err := task.Wait(ctx)
	if err != nil {
		t.Fatal(err)
	}

	runSession(ctx, t, cc, name, "pkill -x sleep")

	select {
	case <-exitC:
		t.Error("expected the sandbox to keep running after its sleep was killed")
	case <-time.After(2 * time.Second):
	}
}

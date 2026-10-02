//go:build integration

package sandbox

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/containerd/containerd/api/services/tasks/v1"
	containerd "github.com/containerd/containerd/v2/client"
	"github.com/containerd/containerd/v2/pkg/oci"
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

	out := runSession(ctx, t, cc, name, "pkill -x sleep && echo killed")
	if !strings.Contains(out, "killed") {
		t.Fatalf("expected pkill to kill the sleep of init, got %q", out)
	}

	select {
	case <-exitC:
		t.Error("expected the sandbox to keep running after its sleep was killed")
	case <-time.After(2 * time.Second):
	}
}

// assertRunning verifies that the sandbox has a running task and returns it.
func assertRunning(
	ctx context.Context, t *testing.T, cc *containerd.Client, name string,
) containerd.Task {
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
	if err != nil {
		t.Fatal(err)
	}

	if status.Status != containerd.Running {
		t.Errorf("task status = %s, want %s", status.Status, containerd.Running)
	}

	return task
}

func TestStartStoppedSandbox(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "start-stopped-test"
	startTestSandbox(ctx, t, cc, name)

	runSession(ctx, t, cc, name, "echo hi > /root/marker")

	if err := Stop(ctx, cc, name, stopTimeout); err != nil {
		t.Fatal(err)
	}

	if err := Start(ctx, cc, name); err != nil {
		t.Fatal(err)
	}

	assertRunning(ctx, t, cc, name)

	out := runSession(ctx, t, cc, name, "cat /root/marker")
	if !strings.Contains(out, "hi") {
		t.Errorf("expected the marker file to survive the restart, got %q", out)
	}
}

func TestStartSandboxWithoutTask(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "start-no-task-test"
	container := startTestSandbox(ctx, t, cc, name)

	task, err := container.Task(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}

	if _, err := task.Delete(ctx, containerd.WithProcessKill); err != nil {
		t.Fatal(err)
	}

	if err := Start(ctx, cc, name); err != nil {
		t.Fatal(err)
	}

	assertRunning(ctx, t, cc, name)
}

func TestStartRunningSandbox(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "start-running-test"
	startTestSandbox(ctx, t, cc, name)

	// The boot ID changes on every boot of the guest kernel, so it only stays
	// the same when the VM kept running.
	bootID := "cat /proc/sys/kernel/random/boot_id"
	before := runSession(ctx, t, cc, name, bootID)

	if err := Start(ctx, cc, name); err != nil {
		t.Fatal(err)
	}

	assertRunning(ctx, t, cc, name)

	if after := runSession(ctx, t, cc, name, bootID); after != before {
		t.Errorf("boot id = %q, want the VM to keep running as %q", after, before)
	}
}

func TestStartUnknownSandbox(t *testing.T) {
	cc := newTestContainerClient(t)

	err := Start(context.Background(), cc, "does-not-exist")
	if !errors.Is(err, ErrSandboxNotFound) {
		t.Fatalf("expected ErrSandboxNotFound, got %v", err)
	}

	if strings.Contains(err.Error(), "could not start sandbox") {
		t.Errorf("expected the not found error without a start prefix, got %v", err)
	}
}

func TestStartReportsBootFailure(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "start-failure-test"
	removeSandbox(cc, name)
	t.Cleanup(func() { removeSandbox(cc, name) })

	// A container with a runtime that doesn't exist can't get a task.
	_, err := cc.NewContainer(ctx, name,
		containerd.WithRuntime("io.containerd.anvil-missing.v1", nil),
		containerd.WithNewSpec(oci.WithProcessArgs("true")),
	)
	if err != nil {
		t.Fatal(err)
	}

	err = Start(ctx, cc, name)

	prefix := fmt.Sprintf("could not start sandbox %q: ", name)
	if err == nil || !strings.HasPrefix(err.Error(), prefix) {
		t.Fatalf("error = %v, want it to start with %q", err, prefix)
	}

	// containerd reports the missing shim binary as an unknown error, which
	// only matches when the cause is wrapped.
	if !errdefs.IsUnknown(err) {
		t.Errorf("expected the error to wrap the containerd cause, got %v", err)
	}

	if errors.Is(err, ErrSandboxNotFound) {
		t.Errorf("expected a boot failure, not a missing sandbox, got %v", err)
	}
}

// initCmdline returns the command line of PID 1 in the sandbox.
func initCmdline(
	ctx context.Context, t *testing.T, cc *containerd.Client, name string,
) string {
	t.Helper()

	out := runSession(ctx, t, cc, name, "tr '\\0' ' ' < /proc/1/cmdline")

	return strings.TrimSpace(out)
}

func TestSandboxRunsTiniAsInit(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "tini-init-test"
	startTestSandbox(ctx, t, cc, name)

	want := strings.Join(initArgs, " ")
	if got := initCmdline(ctx, t, cc, name); got != want {
		t.Errorf("PID 1 = %q, want %q", got, want)
	}
}

func TestInitMountIsReadOnly(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "init-mount-test"
	startTestSandbox(ctx, t, cc, name)

	out := runSession(ctx, t, cc, name,
		"touch /.anvil/x 2>/dev/null && echo writable || echo read-only")
	if !strings.Contains(out, "read-only") {
		t.Errorf("expected /.anvil to be read-only, got %q", out)
	}
}

func TestInitReapsOrphans(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "init-reap-test"
	startTestSandbox(ctx, t, cc, name)

	runSession(ctx, t, cc, name, `sh -c "sleep 1 &"; sleep 3`)

	out := runSession(ctx, t, cc, name,
		"grep -s '^State:' /proc/[0-9]*/status | grep -c Z")
	if got := strings.TrimSpace(out); got != "0" {
		t.Errorf("expected no zombie processes, found %s", got)
	}
}

func TestStartReinstallsInit(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	// A separate init directory keeps the test from wiping the init of
	// sandboxes outside the test.
	dir := filepath.Join(t.TempDir(), "init")
	setInitDir(t, dir)

	name := "init-reinstall-test"
	startTestSandbox(ctx, t, cc, name)

	if err := Stop(ctx, cc, name, stopTimeout); err != nil {
		t.Fatal(err)
	}

	// A host reboot wipes /run/user/<uid>, and the init with it.
	if err := os.RemoveAll(dir); err != nil {
		t.Fatal(err)
	}

	want := strings.Join(initArgs, " ")
	if got := initCmdline(ctx, t, cc, name); got != want {
		t.Errorf("PID 1 = %q, want %q", got, want)
	}
}

func TestStartReportsInitInstallFailure(t *testing.T) {
	cc := newTestContainerClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()

	name := "init-install-failure-test"
	startTestSandbox(ctx, t, cc, name)

	if err := Stop(ctx, cc, name, stopTimeout); err != nil {
		t.Fatal(err)
	}

	// A regular file in the path makes the init directory impossible to create.
	file := filepath.Join(t.TempDir(), "file")
	if err := os.WriteFile(file, nil, 0o600); err != nil {
		t.Fatal(err)
	}

	setInitDir(t, filepath.Join(file, "init"))

	err := Start(ctx, cc, name)
	if !errors.Is(err, syscall.ENOTDIR) {
		t.Fatalf("expected the error to wrap the cause, got %v", err)
	}

	if !errors.Is(err, ErrInitInstallFailed) {
		t.Errorf("expected ErrInitInstallFailed, got %v", err)
	}

	assertTaskRemoved(ctx, t, cc, name)
}

// setInitDir points the sandboxes at another init directory until the test
// finishes.
func setInitDir(t *testing.T, dir string) {
	t.Helper()

	original := initDir
	initDir = func() string { return dir }

	t.Cleanup(func() { initDir = original })
}

package main

import (
	"bytes"
	"context"
	"errors"
	"net"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"testing"

	"github.com/alecthomas/kong"
	"github.com/wmeints/anvil/api/v1alpha1"
	"github.com/wmeints/anvil/internal/control"
	"google.golang.org/grpc"
)

// received holds the requests fakeDaemon got from the CLI.
type received struct {
	created *v1alpha1.CreateSandboxRequest
	removed string
	started *v1alpha1.AttachStart
}

// fakeDaemon records the requests of the CLI and answers them.
type fakeDaemon struct {
	v1alpha1.UnimplementedAnvilServiceServer

	mu  sync.Mutex
	got received
}

// received returns the requests the daemon got so far.
func (d *fakeDaemon) received() received {
	d.mu.Lock()
	defer d.mu.Unlock()

	return d.got
}

func (d *fakeDaemon) CreateSandbox(
	_ context.Context, req *v1alpha1.CreateSandboxRequest,
) (*v1alpha1.CreateSandboxResponse, error) {
	d.mu.Lock()
	defer d.mu.Unlock()

	d.got.created = req
	return &v1alpha1.CreateSandboxResponse{}, nil
}

func (d *fakeDaemon) ListSandboxes(
	context.Context, *v1alpha1.ListSandboxesRequest,
) (*v1alpha1.ListSandboxesResponse, error) {
	return &v1alpha1.ListSandboxesResponse{
		Sandboxes: []*v1alpha1.SandboxSummary{{Name: "one"}, {Name: "two"}},
	}, nil
}

func (d *fakeDaemon) RemoveSandbox(
	_ context.Context, req *v1alpha1.RemoveSandboxRequest,
) (*v1alpha1.RemoveSandboxResponse, error) {
	d.mu.Lock()
	defer d.mu.Unlock()

	d.got.removed = req.GetName()
	return &v1alpha1.RemoveSandboxResponse{}, nil
}

// sessionExitCode is the exit code fakeDaemon reports for every session.
const sessionExitCode = 3

func (d *fakeDaemon) AttachSandbox(
	stream v1alpha1.AnvilService_AttachSandboxServer,
) error {
	req, err := stream.Recv()
	if err != nil {
		return err
	}

	d.mu.Lock()
	d.got.started = req.GetStart()
	d.mu.Unlock()

	return stream.Send(&v1alpha1.AttachSandboxResponse{
		Msg: &v1alpha1.AttachSandboxResponse_ExitCode{ExitCode: sessionExitCode},
	})
}

// startFakeDaemon serves a fakeDaemon on the socket anvil connects to.
func startFakeDaemon(t *testing.T) *fakeDaemon {
	t.Helper()

	dir := t.TempDir()
	t.Setenv("XDG_RUNTIME_DIR", dir)

	sock := filepath.Join(dir, "anvil", "anvil.sock")
	if err := os.MkdirAll(filepath.Dir(sock), 0o700); err != nil {
		t.Fatal(err)
	}

	listener, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}

	daemon := &fakeDaemon{}
	server := grpc.NewServer()
	v1alpha1.RegisterAnvilServiceServer(server, daemon)

	go func() {
		_ = server.Serve(listener)
	}()

	t.Cleanup(server.Stop)

	return daemon
}

// runCLI parses the arguments like main does and runs the command.
func runCLI(t *testing.T, args ...string) (string, error) {
	t.Helper()

	var cli CLI
	parser, err := kong.New(&cli)
	if err != nil {
		t.Fatal(err)
	}

	kctx, err := parser.Parse(args)
	if err != nil {
		t.Fatal(err)
	}

	var stdout bytes.Buffer
	err = run(kctx, strings.NewReader(""), &stdout)

	return stdout.String(), err
}

func TestLs(t *testing.T) {
	startFakeDaemon(t)

	out, err := runCLI(t, "ls")
	if err != nil {
		t.Fatal(err)
	}

	if want := "one\ntwo\n"; out != want {
		t.Errorf("output = %q, want %q", out, want)
	}
}

func TestCreateUsesDefaultImage(t *testing.T) {
	daemon := startFakeDaemon(t)

	if _, err := runCLI(t, "create", "--name", "demo"); err != nil {
		t.Fatal(err)
	}

	created := daemon.received().created
	if created.GetName() != "demo" || created.GetImage() != "ubuntu:26.04" {
		t.Errorf("unexpected create request: %v", created)
	}
}

func TestRm(t *testing.T) {
	daemon := startFakeDaemon(t)

	if _, err := runCLI(t, "rm", "demo"); err != nil {
		t.Fatal(err)
	}

	if removed := daemon.received().removed; removed != "demo" {
		t.Errorf("removed = %q, want %q", removed, "demo")
	}
}

func TestRunPassesCommandAndExitCode(t *testing.T) {
	daemon := startFakeDaemon(t)

	_, err := runCLI(t, "run", "demo", "sh", "-c", "exit 3")

	var exitErr exitCodeError
	if !errors.As(err, &exitErr) || exitErr.ExitCode() != sessionExitCode {
		t.Fatalf("error = %v, want exit code %d", err, sessionExitCode)
	}

	started := daemon.received().started
	want := []string{"sh", "-c", "exit 3"}
	if started.GetSandbox() != "demo" || !slices.Equal(started.GetArgs(), want) {
		t.Errorf("unexpected start message: %v", started)
	}
}

func TestRunWithoutDaemon(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())

	_, err := runCLI(t, "ls")
	if !errors.Is(err, control.ErrDaemonNotRunning) {
		t.Fatalf("error = %v, want %v", err, control.ErrDaemonNotRunning)
	}
}

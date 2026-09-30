//go:build integration

package daemon

import (
	"os"
	"os/signal"
	"syscall"
	"testing"
	"time"

	"github.com/wmeints/anvil/api/v1alpha1"
	"github.com/wmeints/anvil/internal/control"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

func TestRunServesUntilSignaled(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())

	// Keep SIGTERM from killing the test binary in case it arrives before Run
	// registers its own handler.
	sig := make(chan os.Signal, 1)
	signal.Notify(sig, syscall.SIGTERM)
	t.Cleanup(func() { signal.Stop(sig) })

	done := make(chan error, 1)
	go func() { done <- Run() }()

	client := dialDaemon(t, waitForDaemon(t, done))

	_, err := client.ListSandboxes(t.Context(), &v1alpha1.ListSandboxesRequest{})
	if err != nil {
		t.Fatalf("ListSandboxes failed: %v", err)
	}

	if err := terminateDaemon(t, done); err != nil {
		t.Fatalf("Run returned an error: %v", err)
	}
}

// dialDaemon connects a client to the daemon listening on the given address.
func dialDaemon(t *testing.T, addr string) v1alpha1.AnvilServiceClient {
	t.Helper()

	conn, err := grpc.NewClient(addr,
		grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		t.Fatal(err)
	}

	t.Cleanup(func() { _ = conn.Close() })

	return v1alpha1.NewAnvilServiceClient(conn)
}

// terminateDaemon signals the daemon until Run returns, so a signal sent before
// Run registered its handler doesn't stall the test. It returns the result of Run.
func terminateDaemon(t *testing.T, done <-chan error) error {
	t.Helper()

	deadline := time.After(10 * time.Second)
	for {
		if err := syscall.Kill(os.Getpid(), syscall.SIGTERM); err != nil {
			t.Fatal(err)
		}

		select {
		case err := <-done:
			return err
		case <-time.After(100 * time.Millisecond):
		case <-deadline:
			t.Fatal("Run didn't return after SIGTERM")
		}
	}
}

// waitForDaemon waits until the daemon socket appears and returns its address.
func waitForDaemon(t *testing.T, done <-chan error) string {
	t.Helper()

	deadline := time.After(10 * time.Second)
	for {
		if addr, err := control.DaemonSocketAddress(); err == nil {
			return addr
		}

		select {
		case err := <-done:
			t.Fatalf("Run exited before serving: %v", err)
		case <-deadline:
			t.Fatal("daemon socket didn't appear")
		case <-time.After(50 * time.Millisecond):
		}
	}
}

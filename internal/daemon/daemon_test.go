package daemon

import (
	"errors"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wmeints/anvil/internal/paths"
)

func TestCreateListener(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_RUNTIME_DIR", dir)

	lis := newTestListener(t)

	info, err := os.Stat(filepath.Join(dir, "anvil"))
	if err != nil {
		t.Fatal(err)
	}

	if perm := info.Mode().Perm(); perm != 0o700 {
		t.Errorf("socket directory permissions = %o, want 700", perm)
	}

	conn, err := net.Dial("unix", lis.Addr().String())
	if err != nil {
		t.Fatalf("failed to connect to the listener: %v", err)
	}

	_ = conn.Close()
}

func TestCreateListenerReplacesStaleSocket(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_RUNTIME_DIR", dir)

	sock := filepath.Join(dir, "anvil", "anvil.sock")
	if err := os.MkdirAll(filepath.Dir(sock), 0o700); err != nil {
		t.Fatal(err)
	}

	if err := os.WriteFile(sock, nil, 0o600); err != nil {
		t.Fatal(err)
	}

	lis := newTestListener(t)

	if got := lis.Addr().String(); got != sock {
		t.Errorf("listener address = %q, want %q", got, sock)
	}
}

func TestCreateListenerFailsWhenSocketCannotBeRemoved(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_RUNTIME_DIR", dir)

	// A non-empty directory in place of the socket can't be removed.
	sock := filepath.Join(dir, "anvil", "anvil.sock")
	if err := os.MkdirAll(filepath.Join(sock, "blocker"), 0o700); err != nil {
		t.Fatal(err)
	}

	lis, err := createListener()
	if err == nil {
		_ = lis.Close()
		t.Fatal("expected an error when the old socket can't be removed")
	}

	if !errors.Is(err, ErrStaleSocket) {
		t.Errorf("createListener() error = %v, want %v", err, ErrStaleSocket)
	}

	if !strings.Contains(err.Error(), sock) {
		t.Errorf("error %q doesn't mention the socket path %q", err, sock)
	}
}

func TestCreateListenerWithoutRuntimeDir(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", "")

	lis, err := createListener()
	if err == nil {
		_ = lis.Close()
	}

	if !errors.Is(err, paths.ErrRuntimeDirNotSet) {
		t.Fatalf("createListener() error = %v, want %v", err, paths.ErrRuntimeDirNotSet)
	}
}

func TestCreateListenerFailsWhenSocketDirectoryCannotBeCreated(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_RUNTIME_DIR", dir)

	// A file in place of the socket directory blocks creating it.
	if err := os.WriteFile(filepath.Join(dir, "anvil"), nil, 0o600); err != nil {
		t.Fatal(err)
	}

	lis, err := createListener()
	if err == nil {
		_ = lis.Close()
	}

	if !errors.Is(err, ErrSocketDirectory) {
		t.Fatalf("createListener() error = %v, want %v", err, ErrSocketDirectory)
	}
}

func TestCreateListenerFailsWhenSocketPathIsTooLong(t *testing.T) {
	// Unix socket paths are limited to 108 bytes on Linux.
	dir := filepath.Join(t.TempDir(), strings.Repeat("x", 120))
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}

	t.Setenv("XDG_RUNTIME_DIR", dir)

	lis, err := createListener()
	if err == nil {
		_ = lis.Close()
	}

	if !errors.Is(err, ErrListen) {
		t.Fatalf("createListener() error = %v, want %v", err, ErrListen)
	}
}

func newTestListener(t *testing.T) net.Listener {
	t.Helper()

	lis, err := createListener()
	if err != nil {
		t.Fatal(err)
	}

	t.Cleanup(func() { _ = lis.Close() })

	return lis
}

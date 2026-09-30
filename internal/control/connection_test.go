package control

import (
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/wmeints/anvil/internal/paths"
)

func TestDaemonSocketAddress(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_RUNTIME_DIR", dir)

	sock := filepath.Join(dir, "anvil", "anvil.sock")
	if err := os.MkdirAll(filepath.Dir(sock), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(sock, nil, 0o600); err != nil {
		t.Fatal(err)
	}

	addr, err := DaemonSocketAddress()
	if err != nil {
		t.Fatal(err)
	}

	if want := "unix://" + sock; addr != want {
		t.Errorf("DaemonSocketAddress() = %q, want %q", addr, want)
	}
}

func TestDaemonSocketAddressWithoutRuntimeDir(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", "")

	if _, err := DaemonSocketAddress(); !errors.Is(err, paths.ErrRuntimeDirNotSet) {
		t.Fatalf("error = %v, want %v", err, paths.ErrRuntimeDirNotSet)
	}
}

func TestDaemonSocketAddressWithoutDaemon(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())

	if _, err := DaemonSocketAddress(); !errors.Is(err, ErrDaemonNotRunning) {
		t.Fatalf("error = %v, want %v", err, ErrDaemonNotRunning)
	}
}

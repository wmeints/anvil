package paths

import (
	"errors"
	"os"
	"path/filepath"
	"strconv"
	"testing"
)

func TestDaemonSocket(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_RUNTIME_DIR", dir)

	sock, err := DaemonSocket()
	if err != nil {
		t.Fatal(err)
	}

	if want := filepath.Join(dir, "anvil", "anvil.sock"); sock != want {
		t.Errorf("DaemonSocket() = %q, want %q", sock, want)
	}
}

func TestDaemonSocketWithoutRuntimeDir(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", "")

	if _, err := DaemonSocket(); !errors.Is(err, ErrRuntimeDirNotSet) {
		t.Fatalf("DaemonSocket() error = %v, want %v", err, ErrRuntimeDirNotSet)
	}
}

func TestUserPaths(t *testing.T) {
	runDir := filepath.Join("/run/user", strconv.Itoa(os.Getuid()))

	if got, want := ContainerRuntimeSocket(),
		filepath.Join(runDir, "containerd/containerd.sock"); got != want {
		t.Errorf("ContainerRuntimeSocket() = %q, want %q", got, want)
	}

	if got, want := FIFODir(), filepath.Join(runDir, "anvil/fifo"); got != want {
		t.Errorf("FIFODir() = %q, want %q", got, want)
	}

	if got, want := InitDir(), filepath.Join(runDir, "anvil/init"); got != want {
		t.Errorf("InitDir() = %q, want %q", got, want)
	}
}

package sandbox

import (
	"bytes"
	"errors"
	"io/fs"
	"os"
	"path/filepath"

	"github.com/containerd/containerd/v2/pkg/oci"
	"github.com/opencontainers/runtime-spec/specs-go"

	"github.com/wmeints/anvil/internal/paths"
)

// ErrInitInstallFailed is returned when the init binary can't be written to the
// host before a sandbox boots.
var ErrInitInstallFailed = errors.New("could not install the sandbox init")

// initMountPath is where the directory with the init binary is mounted in the
// sandbox.
const initMountPath = "/.anvil"

// initDir returns the host directory with the init binary. Tests replace it to
// point the sandboxes at another directory.
var initDir = paths.InitDir

// initArgs is the init process of a sandbox. tini runs as PID 1, reaps
// orphaned processes and forwards SIGTERM to the shell, which isn't PID 1 and
// so exits on it. The loop restarts the sleep when a process in the sandbox
// kills it, so only a signal ends init.
var initArgs = []string{
	initMountPath + "/tini", "--",
	"/bin/sh", "-c", "while :; do sleep infinity & wait; done",
}

// withInitMount mounts the directory with the init binary read-only in the
// sandbox. The directory is mounted rather than the file, so the binary can be
// replaced on the host without leaving the sandbox with a stale file handle.
func withInitMount() oci.SpecOpts {
	return oci.WithMounts([]specs.Mount{{
		Type:        "bind",
		Source:      initDir(),
		Destination: initMountPath,
		Options:     []string{"rbind", "ro"},
	}})
}

// installInit writes the embedded tini binary to dir, unless the same binary
// is already there. The host wipes the directory on a reboot, so it's installed
// before every boot of a sandbox.
func installInit(dir string) error {
	current, err := fs.ReadFile(os.DirFS(dir), "tini")
	if err == nil && bytes.Equal(current, tini) {
		return nil
	}

	if err := os.MkdirAll(dir, 0o700); err != nil {
		return err
	}

	return writeInit(dir, filepath.Join(dir, "tini"))
}

// writeInit writes the binary to a temporary file and renames it to path, so a
// sandbox booting at the same time never sees a partial binary.
func writeInit(dir, path string) error {
	f, err := os.CreateTemp(dir, ".tini-*")
	if err != nil {
		return err
	}

	defer func() { _ = os.Remove(f.Name()) }()

	_, err = f.Write(tini)
	err = errors.Join(err, f.Chmod(0o755), f.Close())
	if err != nil {
		return err
	}

	return os.Rename(f.Name(), path)
}

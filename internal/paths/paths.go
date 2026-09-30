// Package paths provides the well-known file system locations used by anvil.
package paths

import (
	"errors"
	"os"
	"path/filepath"
	"strconv"
)

// ErrRuntimeDirNotSet is returned when the XDG_RUNTIME_DIR environment variable
// isn't set, so there's no location for the daemon socket.
var ErrRuntimeDirNotSet = errors.New("XDG_RUNTIME_DIR is not set")

// DaemonSocket returns the path of the socket the anvil daemon listens on.
func DaemonSocket() (string, error) {
	dir := os.Getenv("XDG_RUNTIME_DIR")
	if dir == "" {
		return "", ErrRuntimeDirNotSet
	}

	return filepath.Join(dir, "anvil", "anvil.sock"), nil
}

// ContainerRuntimeSocket returns the path to the rootless containerd socket.
func ContainerRuntimeSocket() string {
	return filepath.Join(userRunDir(), "containerd/containerd.sock")
}

// FIFODir returns the directory for the task I/O FIFOs. The containerd default,
// /run/containerd/fifo, isn't writable for the rootless client on the host.
func FIFODir() string {
	return filepath.Join(userRunDir(), "anvil/fifo")
}

func userRunDir() string {
	return filepath.Join("/run/user", strconv.Itoa(os.Getuid()))
}

// Package utils provides shared helpers, such as well-known file system paths.
package utils

import (
	"os/user"
	"path/filepath"
)

// ContainerRuntimeSocketPath returns the path to the rootless containerd socket.
func ContainerRuntimeSocketPath() string {
	currentUser, _ := user.Current()
	return filepath.Join("/run/user/", currentUser.Uid, "containerd/containerd.sock")
}

// FIFODirPath returns the directory for the task I/O FIFOs. The containerd default,
// /run/containerd/fifo, isn't writable for the rootless client on the host.
func FIFODirPath() string {
	currentUser, _ := user.Current()
	return filepath.Join("/run/user/", currentUser.Uid, "anvil/fifo")
}

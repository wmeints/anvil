package control

import (
	"fmt"
	"os"
	"path/filepath"
)

// DaemonSocketAddress finds and returns the socket to communicate with the daemon
// process.
//
// This method returns an error when the XDG_RUNTIME_DIR environment variable isn't set.
// It also returns an error when the socket doesn't exist.
func DaemonSocketAddress() (string, error) {
	dir := os.Getenv("XDG_RUNTIME_DIR")
	if dir == "" {
		return "", fmt.Errorf("XDG_RUNTIME_DIR is not set")
	}

	socketFileName := filepath.Join(dir, "anvil", "anvil.sock")
	_, err := os.Stat(socketFileName)
	if err != nil {
		return "", fmt.Errorf("socket %s does not exist", socketFileName)
	}

	return "unix://" + socketFileName, nil
}

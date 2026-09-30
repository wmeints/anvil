package control

import (
	"errors"
	"fmt"
	"os"

	"github.com/wmeints/anvil/internal/paths"
)

// ErrDaemonNotRunning is returned when the socket of the daemon doesn't exist.
var ErrDaemonNotRunning = errors.New("anvil daemon is not running")

// DaemonSocketAddress finds and returns the socket to communicate with the daemon
// process.
//
// This method returns an error when the XDG_RUNTIME_DIR environment variable isn't
// set. It also returns an error when the socket doesn't exist.
func DaemonSocketAddress() (string, error) {
	sock, err := paths.DaemonSocket()
	if err != nil {
		return "", err
	}

	if _, err := os.Stat(sock); err != nil {
		return "", fmt.Errorf(
			"%w: %s (start it with anvild) %w", ErrDaemonNotRunning, sock, err)
	}

	return "unix://" + sock, nil
}

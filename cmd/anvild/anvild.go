// Command anvild is the daemon that runs and manages anvil sandboxes.
package main

import (
	"log/slog"

	"github.com/wmeints/anvil/internal/daemon"
)

func main() {
	slog.Info("Starting daemon")

	err := daemon.Run()
	if err != nil {
		panic(err)
	}
}

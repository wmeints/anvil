package main

import (
	"context"
	"errors"
	"fmt"
	"time"
)

// errInvalidTimeout is returned when the stop timeout isn't positive.
var errInvalidTimeout = errors.New("invalid --timeout")

// StopCmd hibernates a sandbox: it stops its VM and keeps its files.
type StopCmd struct {
	Name    string        `arg:"" help:"name of sandbox to stop"`
	Timeout time.Duration `default:"10s" help:"time for processes to exit before kill"`
}

// Validate rejects a timeout that isn't positive before the daemon is called.
func (cmd *StopCmd) Validate() error {
	if cmd.Timeout <= 0 {
		return fmt.Errorf("%w: %s (use a duration above zero, such as 10s)",
			errInvalidTimeout, cmd.Timeout)
	}

	return nil
}

// Run executes the stop command.
func (cmd *StopCmd) Run(ctx *Context) error {
	return ctx.client.StopSandbox(context.Background(), cmd.Name, cmd.Timeout)
}

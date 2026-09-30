package main

import (
	"context"
	"fmt"
	"os"
	"os/signal"

	"github.com/containerd/console"
	"github.com/wmeints/anvil/internal/control"
	"golang.org/x/sys/unix"
)

// exitCodeError carries the exit code of a session so anvil exits with it.
type exitCodeError struct {
	code int
}

func (err exitCodeError) Error() string {
	return fmt.Sprintf("command exited with code %d", err.code)
}

// ExitCode returns the exit code of the session command.
func (err exitCodeError) ExitCode() int {
	return err.code
}

// RunCmd runs a command in a sandbox with the terminal attached to it.
type RunCmd struct {
	Sandbox string   `arg:"" help:"name of the sandbox"`
	Command []string `arg:"" passthrough:"" help:"command and arguments to run"`
}

// Run executes the run command.
func (cmd *RunCmd) Run(ctx *Context) error {
	opts := control.SessionOptions{
		Sandbox: cmd.Sandbox,
		Args:    cmd.Command,
		Stdin:   os.Stdin,
		Stdout:  os.Stdout,
	}

	con, err := console.ConsoleFromFile(os.Stdin)
	if err == nil {
		restore, err := attachConsole(con, &opts)
		if err != nil {
			return err
		}

		defer restore()
	}

	code, err := ctx.client.RunSession(context.Background(), opts)
	if err != nil {
		return err
	}

	if code != 0 {
		return exitCodeError{code: code}
	}

	return nil
}

// attachConsole switches the terminal to raw mode and reports its size and
// size changes to the session. The returned function restores the terminal.
func attachConsole(con console.Console, opts *control.SessionOptions) (func(), error) {
	if err := con.SetRaw(); err != nil {
		return nil, fmt.Errorf("could not set terminal to raw mode: %w", err)
	}

	resize := make(chan control.WindowSize, 1)
	opts.Resize = resize
	opts.Size = consoleSize(con)

	winch := make(chan os.Signal, 1)
	signal.Notify(winch, unix.SIGWINCH)

	go func() {
		for range winch {
			resize <- consoleSize(con)
		}
	}()

	return func() {
		signal.Stop(winch)
		_ = con.Reset()
	}, nil
}

func consoleSize(con console.Console) control.WindowSize {
	size, err := con.Size()
	if err != nil {
		return control.WindowSize{}
	}

	return control.WindowSize{Width: uint32(size.Width), Height: uint32(size.Height)}
}

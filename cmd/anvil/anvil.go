// Command anvil is the CLI for creating and managing coding-agent sandboxes.
package main

import (
	"errors"
	"io"
	"os"

	"github.com/alecthomas/kong"
	"github.com/wmeints/anvil/internal/control"
)

// CLI defines the commands of anvil.
type CLI struct {
	Ls     LsCmd     `cmd:"" help:"List all available sandboxes"`
	Create CreateCmd `cmd:"" help:"Create a sandbox"`
	Rm     RmCmd     `cmd:"" help:"Remove a sandbox"`
	Run    RunCmd    `cmd:"" help:"Run a command in a sandbox"`
}

// Context holds the dependencies of the commands.
type Context struct {
	client *control.Client
	stdout io.Writer
}

func main() {
	var cli CLI
	kctx := kong.Parse(&cli, kong.UsageOnError())

	err := run(kctx, os.Stdout)

	// Pass the exit code of a session through without reporting an error.
	var exitErr exitCodeError
	if errors.As(err, &exitErr) {
		os.Exit(exitErr.code)
	}

	kctx.FatalIfErrorf(err)
}

// run connects to the daemon and runs the parsed command. It connects after
// parsing, so --help works without a running daemon.
func run(kctx *kong.Context, stdout io.Writer) error {
	socketAddress, err := control.DaemonSocketAddress()
	if err != nil {
		return err
	}

	client, err := control.New(socketAddress)
	if err != nil {
		return err
	}

	defer func() {
		_ = client.Close()
	}()

	return kctx.Run(&Context{client: client, stdout: stdout})
}

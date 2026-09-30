// Command anvil is the CLI for creating and managing coding-agent sandboxes.
package main

import (
	"errors"
	"os"

	"github.com/alecthomas/kong"
	"github.com/wmeints/anvil/internal/control"
)

var cli struct {
	Ls     LsCmd     `cmd:"" help:"List all available sandboxes"`
	Create CreateCmd `cmd:"" help:"Create a sandbox"`
	Rm     RmCmd     `cmd:"" help:"Remove a sandbox"`
	Run    RunCmd    `cmd:"" help:"Run a command in a sandbox"`
}

type Context struct {
	client *control.Client
}

func main() {
	socketAddress, err := control.DaemonSocketAddress()
	if err != nil {
		panic(err)
	}

	client, err := control.New(socketAddress)
	if err != nil {
		panic(err)
	}

	ctx := kong.Parse(&cli, kong.UsageOnError())
	err = ctx.Run(&Context{
		client: client,
	})

	// Pass the exit code of a session through without reporting an error.
	var exitErr exitCodeError
	if errors.As(err, &exitErr) {
		os.Exit(exitErr.code)
	}

	ctx.FatalIfErrorf(err)
}

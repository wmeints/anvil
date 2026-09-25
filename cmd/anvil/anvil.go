package main

import (
	"context"
	"fmt"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"

	"github.com/alecthomas/kong"

	"github.com/wmeints/anvil/internal/daemon"
	"github.com/wmeints/anvil/internal/nerdbox"
	"github.com/wmeints/anvil/internal/sandbox"
)

const (
	sandboxName  = "default"
	sandboxImage = "docker.io/library/ubuntu:26.04"
)

// CLI describes the anvil command line.
type CLI struct {
	Run RunCmd `cmd:"" help:"Boot the sandbox and open a shell in it."`
}

// RunCmd boots the sandbox and attaches the terminal to a shell inside it.
type RunCmd struct {
	NerdboxDir string `default:"${nerdbox_dir}" env:"ANVIL_NERDBOX_DIR" help:"Directory with the nerdbox shim, libkrun, kernel and rootfs."`
}

// Run executes the run command.
func (r *RunCmd) Run() (err error) {
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	if err := nerdbox.Check(r.NerdboxDir); err != nil {
		return err
	}

	d, err := daemon.Start(ctx, daemon.Options{NerdboxDir: r.NerdboxDir})
	if err != nil {
		return err
	}
	defer func() {
		if stopErr := d.Stop(); err == nil {
			err = stopErr
		}
	}()

	if err := sandbox.Run(ctx, d.Client(), sandboxName, sandboxImage); err != nil {
		return err
	}
	fmt.Printf("Sandbox %q hibernated.\n", sandboxName)
	return nil
}

func main() {
	home, _ := os.UserHomeDir()
	ctx := kong.Parse(&CLI{},
		kong.Name("anvil"),
		kong.Description("Run coding agents inside a microVM sandbox."),
		kong.Vars{"nerdbox_dir": filepath.Join(home, ".local", "lib", "nerdbox")},
	)
	ctx.FatalIfErrorf(ctx.Run())
}

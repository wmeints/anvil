// Package sandbox boots a sandbox microVM and attaches the terminal to a shell in it.
package sandbox

import (
	"context"
	"fmt"
	"os"
	"os/signal"

	"github.com/containerd/console"
	"github.com/containerd/containerd/v2/client"
	"github.com/containerd/containerd/v2/pkg/cio"
	"github.com/containerd/containerd/v2/pkg/namespaces"
	"github.com/containerd/containerd/v2/pkg/oci"
	"github.com/containerd/errdefs"
	"golang.org/x/sys/unix"

	"github.com/wmeints/anvil/internal/daemon"
	"github.com/wmeints/anvil/internal/nerdbox"
)

const namespace = "anvil"

// Run boots the sandbox called name from the image ref and attaches the
// terminal to /bin/bash inside it. When the shell exits the VM is stopped,
// while the sandbox and its disk are kept so the next Run resumes it.
func Run(ctx context.Context, c *client.Client, name, ref string) error {
	ctx = namespaces.WithNamespace(ctx, namespace)

	container, err := loadOrCreate(ctx, c, name, ref)
	if err != nil {
		return err
	}

	// A previous session that crashed can leave a task behind.
	if task, err := container.Task(ctx, nil); err == nil {
		if _, err := task.Delete(ctx, client.WithProcessKill); err != nil {
			return fmt.Errorf("remove stale task: %w", err)
		}
	}

	con := console.Current()
	if err := con.SetRaw(); err != nil {
		return fmt.Errorf("set terminal to raw mode: %w", err)
	}
	defer func() { _ = con.Reset() }()

	task, err := container.NewTask(ctx, cio.NewCreator(cio.WithStreams(con, con, nil), cio.WithTerminal))
	if err != nil {
		return fmt.Errorf("boot sandbox: %w", err)
	}
	// Deleting the task stops the VM but keeps the container and its snapshot.
	defer func() { _, _ = task.Delete(context.WithoutCancel(ctx), client.WithProcessKill) }()

	statusC, err := task.Wait(ctx)
	if err != nil {
		return err
	}
	if err := task.Start(ctx); err != nil {
		return fmt.Errorf("start shell: %w", err)
	}

	resize := func() {
		if size, err := con.Size(); err == nil {
			_ = task.Resize(ctx, uint32(size.Width), uint32(size.Height))
		}
	}
	resize()
	winch := make(chan os.Signal, 1)
	signal.Notify(winch, unix.SIGWINCH)
	defer signal.Stop(winch)
	go func() {
		for range winch {
			resize()
		}
	}()

	select {
	case status := <-statusC:
		return status.Error()
	case <-ctx.Done():
		return ctx.Err()
	}
}

func loadOrCreate(ctx context.Context, c *client.Client, name, ref string) (client.Container, error) {
	container, err := c.LoadContainer(ctx, name)
	if err == nil {
		return container, nil
	}
	if !errdefs.IsNotFound(err) {
		return nil, err
	}

	image, err := c.GetImage(ctx, ref)
	if errdefs.IsNotFound(err) {
		fmt.Printf("Pulling %s...\n", ref)
		image, err = c.Pull(ctx, ref, client.WithPullUnpack, client.WithPullSnapshotter(daemon.Snapshotter))
	}
	if err != nil {
		return nil, fmt.Errorf("get image %s: %w", ref, err)
	}

	container, err = c.NewContainer(ctx, name,
		client.WithImage(image),
		client.WithSnapshotter(daemon.Snapshotter),
		client.WithNewSnapshot(name+"-snapshot", image),
		client.WithRuntime(nerdbox.RuntimeName, nil),
		client.WithNewSpec(
			oci.WithImageConfig(image),
			oci.WithTTY,
			oci.WithProcessArgs("/bin/bash"),
			oci.WithHostname("anvil"),
		),
	)
	if err != nil {
		return nil, fmt.Errorf("create sandbox: %w", err)
	}
	return container, nil
}

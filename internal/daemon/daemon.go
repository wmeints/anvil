// Package daemon runs a private containerd instance that anvil uses to manage
// its sandboxes.
package daemon

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"time"

	"github.com/containerd/containerd/v2/client"
)

const (
	startTimeout = 10 * time.Second
	stopTimeout  = 10 * time.Second

	// writableLayerSize is the size of the ext4 disk image that holds a
	// sandbox's changes on top of its image.
	writableLayerSize = "20G"
)

// The erofs snapshotter and differ turn image layers into erofs image files
// and give each container an ext4 disk image, so nothing is mounted on the host
// and containerd runs without root.
const configTemplate = `version = 3
root = %[1]q
state = %[2]q
disabled_plugins = ["io.containerd.grpc.v1.cri", "io.containerd.nri.v1.nri"]

[grpc]
  address = %[3]q
  uid = %[5]d
  gid = %[6]d

[ttrpc]
  address = "%[3]s.ttrpc"
  uid = %[5]d
  gid = %[6]d

[plugins."io.containerd.internal.v1.opt"]
  path = %[4]q

[plugins."io.containerd.snapshotter.v1.erofs"]
  default_size = %[7]q
  dmverity_mode = "off"

[plugins."io.containerd.service.v1.diff-service"]
  default = ["erofs", "walking"]
`

// Snapshotter is the containerd snapshotter sandboxes must use.
const Snapshotter = "erofs"

// paths holds the locations anvil's containerd uses, all owned by the user.
type paths struct {
	root, state, opt, socket, config, log string
}

func newPaths() (paths, error) {
	runtimeDir := os.Getenv("XDG_RUNTIME_DIR")
	if runtimeDir == "" {
		return paths{}, errors.New("XDG_RUNTIME_DIR is not set; anvil needs it for its runtime state")
	}
	dataDir := os.Getenv("XDG_DATA_HOME")
	if dataDir == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			return paths{}, err
		}
		dataDir = filepath.Join(home, ".local", "share")
	}

	data := filepath.Join(dataDir, "anvil")
	run := filepath.Join(runtimeDir, "anvil")
	return paths{
		root:   filepath.Join(data, "containerd"),
		opt:    filepath.Join(data, "opt"),
		log:    filepath.Join(data, "containerd.log"),
		state:  filepath.Join(run, "containerd"),
		socket: filepath.Join(run, "containerd.sock"),
		config: filepath.Join(run, "containerd.toml"),
	}, nil
}

// Options configures the private containerd instance.
type Options struct {
	// NerdboxDir holds the nerdbox shim, libkrun, guest kernel and rootfs.
	NerdboxDir string
}

// Daemon is a connection to anvil's containerd, which it may own.
type Daemon struct {
	client *client.Client
	cmd    *exec.Cmd
	exited chan struct{}
}

// Start connects to anvil's containerd, starting it when it isn't running yet.
func Start(ctx context.Context, opts Options) (*Daemon, error) {
	p, err := newPaths()
	if err != nil {
		return nil, err
	}

	if c, err := connect(ctx, p.socket); err == nil {
		return &Daemon{client: c}, nil
	}

	for _, dir := range []string{p.root, p.opt, p.state} {
		if err := os.MkdirAll(dir, 0o700); err != nil {
			return nil, err
		}
	}
	config := fmt.Sprintf(configTemplate, p.root, p.state, p.socket, p.opt, os.Getuid(), os.Getgid(), writableLayerSize)
	if err := os.WriteFile(p.config, []byte(config), 0o600); err != nil {
		return nil, err
	}
	logFile, err := os.OpenFile(p.log, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o600)
	if err != nil {
		return nil, err
	}
	defer func() { _ = logFile.Close() }()

	cmd := exec.Command("containerd", "--config", p.config)
	cmd.Env = append(os.Environ(),
		"PATH="+opts.NerdboxDir+string(os.PathListSeparator)+os.Getenv("PATH"),
		"LIBKRUN_PATH="+opts.NerdboxDir,
	)
	cmd.Stdout = logFile
	cmd.Stderr = logFile
	// Own process group, so signals sent to the terminal don't reach containerd.
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := cmd.Start(); err != nil {
		return nil, fmt.Errorf("start containerd: %w", err)
	}

	d := &Daemon{cmd: cmd, exited: make(chan struct{})}
	go func() {
		_ = cmd.Wait()
		close(d.exited)
	}()

	deadline := time.Now().Add(startTimeout)
	for {
		select {
		case <-d.exited:
			return nil, fmt.Errorf("containerd exited during startup, see %s", p.log)
		case <-ctx.Done():
			_ = d.Stop()
			return nil, ctx.Err()
		case <-time.After(100 * time.Millisecond):
		}
		if c, err := connect(ctx, p.socket); err == nil {
			d.client = c
			return d, nil
		}
		if time.Now().After(deadline) {
			_ = d.Stop()
			return nil, fmt.Errorf("containerd did not become ready within %s, see %s", startTimeout, p.log)
		}
	}
}

// connect returns a client when containerd is serving on the socket.
func connect(ctx context.Context, socket string) (*client.Client, error) {
	if _, err := os.Stat(socket); err != nil {
		return nil, err
	}
	c, err := client.New(socket)
	if err != nil {
		return nil, err
	}
	ctx, cancel := context.WithTimeout(ctx, time.Second)
	defer cancel()
	if _, err := c.IsServing(ctx); err != nil {
		_ = c.Close()
		return nil, err
	}
	return c, nil
}

// Client returns the containerd client.
func (d *Daemon) Client() *client.Client {
	return d.client
}

// Stop closes the client and, when this Daemon started containerd, stops it.
func (d *Daemon) Stop() error {
	if d.client != nil {
		_ = d.client.Close()
	}
	if d.cmd == nil {
		return nil
	}

	if err := d.cmd.Process.Signal(syscall.SIGTERM); err != nil {
		return err
	}
	select {
	case <-d.exited:
		return nil
	case <-time.After(stopTimeout):
		return d.cmd.Process.Kill()
	}
}

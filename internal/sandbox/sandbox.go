// Package sandbox manages the runtime for the sandboxes in the application.
// A sandbox is a long-lived virtual machine-backed container running in the
// `anvil` namespace in containerd.
package sandbox

import (
	"context"
	"crypto/rand"
	"errors"
	"fmt"
	"regexp"
	"strings"
	"syscall"
	"time"

	containerd "github.com/containerd/containerd/v2/client"
	"github.com/containerd/containerd/v2/core/containers"
	"github.com/containerd/containerd/v2/pkg/cio"
	"github.com/containerd/containerd/v2/pkg/oci"
	"github.com/containerd/errdefs"
	"github.com/distribution/reference"
)

// ErrSandboxNotFound is returned when a sandbox doesn't exist.
var ErrSandboxNotFound = errors.New("sandbox not found")

// ErrInvalidName is returned when a sandbox name isn't lower case letters and
// digits, starting with a letter, optionally separated by single dashes.
var ErrInvalidName = errors.New("invalid sandbox name")

// ErrInvalidImage is returned when a sandbox has no image.
var ErrInvalidImage = errors.New("invalid sandbox image")

// Sandbox defines the control structure for a single sandbox container
type Sandbox struct {
	Name   string `yaml:"name"`
	Spec   Spec   `yaml:"spec"`
	Status Status `yaml:"status"`
}

// Spec defines the specification used to run the sandbox container
type Spec struct {
	Image string `yaml:"image"`
}

// Status models the runtime status of the container.
//
// This struct is used to record the desired state of the container not the
// actual state in the containerd environment. When the containerd state differs
// from the desired state, we'll reconsile the containerd state based on what
// we expected here.
type Status struct {
}

// isValidContainerName validates the name given for the container.
// We expect a container name to be lower case letters and digits, optionally
// separated by single dashes. Names can't start with a digit.
func isValidContainerName(name string) bool {
	m, _ := regexp.Match("^[a-z][a-z0-9]*(-[a-z0-9]+)*$", []byte(name))
	return m
}

// containerLabels specifies the labels that we need to put on the sandbox container
// so we can recognize it later when we need to interact with the container.
func containerLabels() map[string]string {
	return map[string]string{
		"com.github.wmeints.anvil": "true",
	}
}

// withImageEnv applies the image's environment and working directory and runs
// the process as root. oci.WithImageConfig would also look up the user's groups
// in the image, which mounts the rootfs on the host and fails without root.
func withImageEnv(image containerd.Image) oci.SpecOpts {
	return func(
		ctx context.Context, cl oci.Client, c *containers.Container, s *oci.Spec,
	) error {
		spec, err := image.Spec(ctx)
		if err != nil {
			return err
		}

		cwd := spec.Config.WorkingDir
		if cwd == "" {
			cwd = "/"
		}

		return oci.Compose(
			oci.WithEnv(spec.Config.Env),
			oci.WithProcessCwd(cwd),
			oci.WithUIDGID(0, 0),
		)(ctx, cl, c, s)
	}
}

// initArgs is the init process of a sandbox. It keeps the sandbox running
// until it gets SIGTERM. Init ignores signals it doesn't handle, so a plain
// `sleep infinity` would only stop when it's killed. The loop restarts the
// sleep when a process in the sandbox kills it, so only SIGTERM ends init.
var initArgs = []string{
	"/bin/sh", "-c", "trap 'exit 0' TERM; while :; do sleep infinity & wait; done",
}

// Start boots the container for the sandbox and keeps it running.
func (sb *Sandbox) Start(ctx context.Context, cc *containerd.Client) error {
	return sb.start(ctx, cc, initArgs)
}

// start creates the container for the sandbox with the given init process and
// boots it.
func (sb *Sandbox) start(
	ctx context.Context, cc *containerd.Client, args []string,
) error {
	imageRef, err := reference.ParseDockerRef(sb.Spec.Image)
	if err != nil {
		return err
	}

	image, err := cc.Pull(ctx, imageRef.String(), containerd.WithPullUnpack)
	if err != nil {
		return err
	}

	// A random suffix keeps the snapshot key unique, so a leftover snapshot
	// from an earlier sandbox with the same name doesn't block creation.
	suffix := strings.ToLower(rand.Text()[:8])
	snapshotName := fmt.Sprintf("%s-snapshot-%s", sb.Name, suffix)

	container, err := cc.NewContainer(ctx, sb.Name,
		containerd.WithNewSnapshot(snapshotName, image),
		containerd.WithRuntime("io.containerd.nerdbox.v1", nil),
		containerd.WithContainerLabels(containerLabels()),
		containerd.WithNewSpec(
			withImageEnv(image),
			oci.WithProcessArgs(args...),
		),
	)
	if err != nil {
		return err
	}

	_, err = startTask(ctx, container)

	return err
}

// startTask boots the VM of the sandbox by creating and starting a new task for
// its container.
func startTask(
	ctx context.Context, container containerd.Container,
) (containerd.Task, error) {
	task, err := container.NewTask(ctx, cio.NullIO)
	if err != nil {
		return nil, err
	}

	if err := task.Start(ctx); err != nil {
		_, _ = task.Delete(context.WithoutCancel(ctx), containerd.WithProcessKill)
		return nil, err
	}

	return task, nil
}

// Stop hibernates the sandbox: it asks the processes in the sandbox to
// terminate, kills them when the init process hasn't exited within the timeout,
// and stops the VM. The container and its disk are kept, so the next session
// boots the sandbox again with its files intact. A timeout of zero or less
// kills the processes right away.
func Stop(
	ctx context.Context, cc *containerd.Client, name string, timeout time.Duration,
) error {
	container, err := loadContainer(ctx, cc, name)
	if err != nil {
		return err
	}

	if err := stopTask(ctx, container, timeout); err != nil {
		return fmt.Errorf("could not stop sandbox %q: %w", name, err)
	}

	return nil
}

// Remove stops the VM of the sandbox and deletes the container together with
// its snapshot.
func Remove(ctx context.Context, cc *containerd.Client, name string) error {
	container, err := loadContainer(ctx, cc, name)
	if err != nil {
		return err
	}

	if err := stopTask(ctx, container, 0); err != nil {
		return fmt.Errorf("could not stop sandbox %q: %w", name, err)
	}

	if err := container.Delete(ctx, containerd.WithSnapshotCleanup); err != nil {
		return fmt.Errorf("could not remove sandbox %q: %w", name, err)
	}

	return nil
}

// loadContainer returns the container of the sandbox.
func loadContainer(
	ctx context.Context, cc *containerd.Client, name string,
) (containerd.Container, error) {
	container, err := cc.LoadContainer(ctx, name)
	if err != nil {
		return nil, fmt.Errorf("%w: %s (check the name with anvil ls) %w",
			ErrSandboxNotFound, name, err)
	}

	return container, nil
}

// stopTask terminates the processes of the sandbox within the timeout and
// deletes its task. Deleting the task stops the shim, which terminates the VM.
// A sandbox whose VM already stopped has no task left to delete.
func stopTask(
	ctx context.Context, container containerd.Container, timeout time.Duration,
) error {
	task, err := container.Task(ctx, nil)
	if err == nil && timeout > 0 {
		err = terminate(ctx, task, timeout)
	}

	// The processes already got SIGTERM, so the task is deleted even when the
	// caller gives up, rather than leaving the sandbox half stopped.
	if err == nil {
		_, err = task.Delete(context.WithoutCancel(ctx), containerd.WithProcessKill)
	}

	if errdefs.IsNotFound(err) {
		return nil
	}

	return err
}

// termInterval is how often stop repeats SIGTERM for the init process.
const termInterval = 250 * time.Millisecond

// terminate sends SIGTERM to the processes of a running task and waits up to
// the timeout for its init process to exit.
func terminate(ctx context.Context, task containerd.Task, timeout time.Duration) error {
	status, err := task.Status(ctx)
	if err != nil || status.Status == containerd.Stopped {
		return err
	}

	waitCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()

	exitC, err := task.Wait(waitCtx)
	if err != nil {
		return err
	}

	if err := signalTerm(ctx, task, containerd.WithKillAll); err != nil {
		return err
	}

	awaitExit(ctx, task, exitC, waitCtx.Done())

	return nil
}

// awaitExit waits for the init process of the task to exit until done closes.
// A freshly booted init process ignores SIGTERM until it has set up its
// handler, so the signal is repeated for the init process while it runs. The
// repeats are best effort: a failure, such as for an init process that just
// exited or a cancelled caller, must not keep the task from being deleted.
func awaitExit(
	ctx context.Context, task containerd.Task,
	exitC <-chan containerd.ExitStatus, done <-chan struct{},
) {
	ticker := time.NewTicker(termInterval)
	defer ticker.Stop()

	for {
		select {
		case <-exitC:
			return
		case <-done:
			return
		case <-ticker.C:
			_ = signalTerm(ctx, task)
		}
	}
}

// signalTerm sends SIGTERM to the task. A task that exited in the meantime has
// no processes left to signal.
func signalTerm(
	ctx context.Context, task containerd.Task, opts ...containerd.KillOpts,
) error {
	err := task.Kill(ctx, syscall.SIGTERM, opts...)
	if errdefs.IsNotFound(err) {
		return nil
	}

	return err
}

// NewSandbox creates a new sandbox definition
// Use the operations like Start/Stop/PullImage to manage the sandbox instance.
func NewSandbox(name string, image string) (*Sandbox, error) {
	if !isValidContainerName(name) {
		return nil, fmt.Errorf(
			"%w: %q (use lower case letters, digits and dashes; start with a letter)",
			ErrInvalidName, name)
	}

	if image == "" {
		return nil, fmt.Errorf("%w (pass an image such as ubuntu:26.04)",
			ErrInvalidImage)
	}

	sandboxSpec := Spec{
		Image: image,
	}

	return &Sandbox{
		Name: name,
		Spec: sandboxSpec,
	}, nil
}

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
// digits, optionally separated by dashes, starting with a letter.
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
// We expect a container name to be all lower case, optionally containing dashes.
// You can number containers, but they can't start with a number.
func isValidContainerName(name string) bool {
	m, _ := regexp.Match("^[a-z]+(-[a-z0-9]+)*$", []byte(name))
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

// Start boots the container for the sandbox and keeps it running.
func (sb *Sandbox) Start(ctx context.Context, cc *containerd.Client) error {
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
			oci.WithProcessArgs("sleep", "infinity"),
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

// Remove stops the VM of the sandbox and deletes the container together with
// its snapshot.
func Remove(ctx context.Context, cc *containerd.Client, name string) error {
	container, err := cc.LoadContainer(ctx, name)
	if err != nil {
		return fmt.Errorf("%w: %s (check the name with anvil ls) %w",
			ErrSandboxNotFound, name, err)
	}

	// Deleting the task stops the shim, which terminates the VM. A sandbox
	// whose VM already stopped has no task left to delete.
	task, err := container.Task(ctx, nil)
	if err == nil {
		_, err = task.Delete(ctx, containerd.WithProcessKill)
	}

	if err != nil && !errdefs.IsNotFound(err) {
		return fmt.Errorf("could not stop sandbox %q: %w", name, err)
	}

	if err := container.Delete(ctx, containerd.WithSnapshotCleanup); err != nil {
		return fmt.Errorf("could not remove sandbox %q: %w", name, err)
	}

	return nil
}

// NewSandbox creates a new sandbox definition
// Use the operations like Start/Stop/PullImage to manage the sandbox instance.
func NewSandbox(name string, image string) (*Sandbox, error) {
	if !isValidContainerName(name) {
		return nil, fmt.Errorf("%w: %q (use lower case letters, digits and dashes)",
			ErrInvalidName, name)
	}

	if image == "" {
		return nil, ErrInvalidImage
	}

	sandboxSpec := Spec{
		Image: image,
	}

	return &Sandbox{
		Name: name,
		Spec: sandboxSpec,
	}, nil
}

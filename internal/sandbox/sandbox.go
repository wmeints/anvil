// Package sandbox manages the runtime for the sandboxes in the application.
// A sandbox is a long-lived virtual machine-backed container running in the
// `anvil` namespace in containerd.
package sandbox

import (
	"context"
	"crypto/rand"
	"errors"
	"fmt"
	"maps"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"time"

	containerd "github.com/containerd/containerd/v2/client"
	"github.com/containerd/containerd/v2/core/containers"
	"github.com/containerd/containerd/v2/pkg/cio"
	"github.com/containerd/containerd/v2/pkg/oci"
	"github.com/containerd/errdefs"
	"github.com/distribution/reference"
	ocispec "github.com/opencontainers/image-spec/specs-go/v1"
	"github.com/opencontainers/runtime-spec/specs-go"
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

// maxNameLength is the longest sandbox name. The name is the hostname of the
// sandbox, and the kernel rejects hostnames longer than 64 characters.
const maxNameLength = 63

// isValidContainerName validates the name given for the container.
// We expect a container name to be lower case letters and digits, optionally
// separated by single dashes. Names can't start with a digit.
func isValidContainerName(name string) bool {
	m, _ := regexp.Match("^[a-z][a-z0-9]*(-[a-z0-9]+)*$", []byte(name))
	return m && len(name) <= maxNameLength
}

// containerLabels specifies the labels that we need to put on the sandbox container
// so we can recognize it later when we need to interact with the container.
func containerLabels() map[string]string {
	return map[string]string{
		"com.github.wmeints.anvil": "true",
	}
}

// ErrInvalidImageUser is returned when the image of a sandbox doesn't declare a
// numeric non-root user. Its message tells how to fix the image.
var ErrInvalidImageUser = errors.New(
	"declare a numeric non-root USER such as 1000:1000")

// parseImageUser parses the USER of an image as UID[:GID]. Anvil can't resolve
// user names, because that reads /etc/passwd from the rootfs and mounts it on
// the host. Without a GID, the GID equals the UID.
func parseImageUser(user string) (uint32, uint32, error) {
	if user == "" {
		return 0, 0, fmt.Errorf("has no user; %w", ErrInvalidImageUser)
	}

	uidText, gidText, found := strings.Cut(user, ":")
	if !found {
		gidText = uidText
	}

	uid, uidErr := strconv.ParseUint(uidText, 10, 32)
	gid, gidErr := strconv.ParseUint(gidText, 10, 32)

	if err := errors.Join(uidErr, gidErr); err != nil {
		return 0, 0, invalidUserError(user, err)
	}

	if uid == 0 || gid == 0 {
		return 0, 0, fmt.Errorf("user %q is root; %w", user, ErrInvalidImageUser)
	}

	return uint32(uid), uint32(gid), nil
}

// invalidUserError explains why a UID or GID didn't parse as a number.
func invalidUserError(user string, err error) error {
	if errors.Is(err, strconv.ErrSyntax) {
		return fmt.Errorf("user %q is not numeric; %w", user, ErrInvalidImageUser)
	}

	return fmt.Errorf("user %q is out of range; %w", user, ErrInvalidImageUser)
}

// processArgs returns the init process of a sandbox: the image entrypoint
// followed by `sleep infinity`, which keeps the sandbox running. The image Cmd
// is ignored, so entrypoints must `exec "$@"`.
func processArgs(config ocispec.ImageConfig) []string {
	return append(slices.Clone(config.Entrypoint), "sleep", "infinity")
}

// cgroupMount mounts the cgroup filesystem of the container, which dockerd
// needs. containerd's default spec has none. It returns a new mount on every
// call, because spec options such as oci.WithWriteableCgroupfs change the
// options in place.
func cgroupMount() specs.Mount {
	return specs.Mount{
		Destination: "/sys/fs/cgroup",
		Type:        "cgroup",
		Source:      "cgroup",
		Options:     []string{"nosuid", "noexec", "nodev", "relatime", "rw"},
	}
}

// volumeMounts backs each VOLUME of the image with a tmpfs, sorted by path.
// Without it, a volume sits on the virtiofs rootfs, which can't hold the upper
// directory of an overlay, such as Docker's storage in /var/lib/docker. The
// contents live in the memory of the VM and are lost when the sandbox stops.
func volumeMounts(config ocispec.ImageConfig) []specs.Mount {
	mounts := []specs.Mount{}

	for _, dest := range slices.Sorted(maps.Keys(config.Volumes)) {
		mounts = append(mounts, specs.Mount{
			Destination: dest,
			Type:        "tmpfs",
			Source:      "tmpfs",
			Options:     []string{"rw", "nosuid", "nodev"},
		})
	}

	return mounts
}

// withVMPrivileges runs the sandbox container privileged. The microVM isolates
// the sandbox from the host, so the container may use everything inside the
// VM, such as dockerd and sudo. oci.WithPrivileged isn't used, because it
// copies the capabilities of the rootless daemon instead of all known ones.
func withVMPrivileges(
	ctx context.Context, cl oci.Client, c *containers.Container, s *oci.Spec,
) error {
	return oci.Compose(
		oci.WithMounts([]specs.Mount{cgroupMount()}),
		oci.WithAllKnownCapabilities,
		oci.WithNewPrivileges,
		oci.WithMaskedPaths(nil),
		oci.WithReadonlyPaths(nil),
		oci.WithWriteableSysfs,
		oci.WithWriteableCgroupfs,
		oci.WithSeccompUnconfined,
	)(ctx, cl, c, s)
}

// sandboxSpecOpts derives the sandbox process from the image config: its
// environment, working directory, numeric user, entrypoint and volumes. The
// hostname is the sandbox name, because the guest doesn't set one.
// oci.WithImageConfig isn't used, because it looks up the user in the image,
// which mounts the rootfs on the host and fails without root.
func sandboxSpecOpts(name string, config ocispec.ImageConfig) (oci.SpecOpts, error) {
	uid, gid, err := parseImageUser(config.User)
	if err != nil {
		return nil, err
	}

	cwd := config.WorkingDir
	if cwd == "" {
		cwd = "/"
	}

	return oci.Compose(
		oci.WithHostname(name),
		oci.WithEnv(config.Env),
		oci.WithProcessCwd(cwd),
		oci.WithUIDGID(uid, gid),
		oci.WithProcessArgs(processArgs(config)...),
		oci.WithMounts(volumeMounts(config)),
		withVMPrivileges,
	), nil
}

// Start boots the container for the sandbox and keeps it running.
func (sb *Sandbox) Start(ctx context.Context, cc *containerd.Client) error {
	return sb.start(ctx, cc)
}

// start creates the container for the sandbox and boots it. The options are
// applied after the ones derived from the image, so tests can override them.
func (sb *Sandbox) start(
	ctx context.Context, cc *containerd.Client, opts ...oci.SpecOpts,
) error {
	imageRef, err := reference.ParseDockerRef(sb.Spec.Image)
	if err != nil {
		return err
	}

	image, err := cc.Pull(ctx, imageRef.String(), containerd.WithPullUnpack)
	if err != nil {
		return err
	}

	imageOpts, err := sb.imageSpecOpts(ctx, image)
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
		containerd.WithNewSpec(append([]oci.SpecOpts{imageOpts}, opts...)...),
	)
	if err != nil {
		return err
	}

	_, err = startTask(ctx, container)

	return err
}

// imageSpecOpts reads the config of the image and derives the sandbox process
// from it. An image without a valid user is rejected before anvil creates a
// container or snapshot for it.
func (sb *Sandbox) imageSpecOpts(
	ctx context.Context, image containerd.Image,
) (oci.SpecOpts, error) {
	spec, err := image.Spec(ctx)
	if err != nil {
		return nil, fmt.Errorf("could not read the config of image %s: %w",
			sb.Spec.Image, err)
	}

	opts, err := sandboxSpecOpts(sb.Name, spec.Config)
	if err != nil {
		return nil, fmt.Errorf("sandbox %s: image %s %w", sb.Name, sb.Spec.Image, err)
	}

	return opts, nil
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
// and stops the VM. The container and its disk are kept, so Start or the next
// session boots the sandbox again with its files intact. A timeout of zero or
// less kills the processes right away.
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

// Start boots the VM of a sandbox that isn't running, for example after a stop
// or a host reboot. The VM cold-boots on the kept disk, so files survive but
// processes don't. Starting a running sandbox leaves it as it is.
func Start(ctx context.Context, cc *containerd.Client, name string) error {
	_, err := ensureRunning(ctx, cc, name)
	return err
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
		return nil, fmt.Errorf("%w: %q (use at most %d lower case letters, "+
			"digits and dashes; start with a letter)",
			ErrInvalidName, name, maxNameLength)
	}

	if image == "" {
		return nil, fmt.Errorf(
			"%w (pass an image such as ghcr.io/wmeints/anvil-base:latest)",
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

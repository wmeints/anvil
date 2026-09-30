package sandbox

import (
	"context"
	"crypto/rand"
	"fmt"
	"io"
	"strings"

	containerd "github.com/containerd/containerd/v2/client"
	"github.com/containerd/containerd/v2/pkg/cio"
	"github.com/containerd/errdefs"
	"github.com/opencontainers/runtime-spec/specs-go"
	"github.com/wmeints/anvil/internal/paths"
)

// sessionTerm is the terminal type of every session. The host TERM isn't used
// because the sandbox image may not have a terminfo entry for it.
const sessionTerm = "xterm-256color"

// SessionOptions configures the process started for a session.
type SessionOptions struct {
	Args   []string
	Width  uint32
	Height uint32
	Stdin  io.Reader
	Stdout io.Writer
}

// Session is an interactive process running in a sandbox with a terminal
// attached to it.
type Session struct {
	process containerd.Process
	exitC   <-chan containerd.ExitStatus
}

// StartSession runs a new process in the sandbox with a terminal connected to
// the provided streams. A sandbox whose VM isn't running is started first.
func StartSession(
	ctx context.Context, cc *containerd.Client, name string, opts SessionOptions,
) (*Session, error) {
	task, err := ensureRunning(ctx, cc, name)
	if err != nil {
		return nil, err
	}

	spec, err := sessionProcessSpec(ctx, task, opts)
	if err != nil {
		return nil, err
	}

	ioCreator := cio.NewCreator(
		cio.WithStreams(opts.Stdin, opts.Stdout, nil),
		cio.WithTerminal,
		cio.WithFIFODir(paths.FIFODir()),
	)

	execID := "session-" + strings.ToLower(rand.Text()[:8])

	process, err := task.Exec(ctx, execID, spec, ioCreator)
	if err != nil {
		return nil, fmt.Errorf("could not create session in %q: %w", name, err)
	}

	session, err := startProcess(ctx, process)
	if err != nil {
		_, _ = process.Delete(context.WithoutCancel(ctx), containerd.WithProcessKill)
		return nil, err
	}

	// A terminal without a known size keeps the default size.
	_ = session.Resize(ctx, opts.Width, opts.Height)

	return session, nil
}

// Resize changes the terminal size of the session.
func (s *Session) Resize(ctx context.Context, width, height uint32) error {
	return s.process.Resize(ctx, width, height)
}

// Wait blocks until the session process exits and all of its output has been
// written. It returns the exit code of the process.
func (s *Session) Wait() (uint32, error) {
	status := <-s.exitC
	code, _, err := status.Result()

	s.process.IO().Wait()

	return code, err
}

// Close kills the session process if it's still running and removes it.
func (s *Session) Close(ctx context.Context) error {
	_, err := s.process.Delete(ctx, containerd.WithProcessKill)
	return err
}

// ensureRunning returns the running task of the sandbox. It boots the VM of the
// sandbox when it stopped, for example after a hibernate or a host reboot.
func ensureRunning(
	ctx context.Context, cc *containerd.Client, name string,
) (containerd.Task, error) {
	container, err := cc.LoadContainer(ctx, name)
	if err != nil {
		return nil, fmt.Errorf("%w: %s (check the name with anvil ls) %w",
			ErrSandboxNotFound, name, err)
	}

	task, err := container.Task(ctx, nil)
	if errdefs.IsNotFound(err) {
		return startTask(ctx, container)
	}

	if err != nil {
		return nil, err
	}

	return resumeTask(ctx, container, task)
}

// resumeTask gets an existing task of the sandbox running again.
func resumeTask(
	ctx context.Context, container containerd.Container, task containerd.Task,
) (containerd.Task, error) {
	status, err := task.Status(ctx)
	if err != nil {
		return nil, err
	}

	switch status.Status {
	case containerd.Running:
		return task, nil
	case containerd.Created:
		return task, task.Start(ctx)
	case containerd.Paused:
		return task, task.Resume(ctx)
	}

	// A stopped task can't be started again, so it's replaced by a new one.
	_, err = task.Delete(ctx, containerd.WithProcessKill)
	if err != nil && !errdefs.IsNotFound(err) {
		return nil, fmt.Errorf("could not restart sandbox %q: %w", container.ID(), err)
	}

	return startTask(ctx, container)
}

// sessionProcessSpec derives the session process from the sandbox process so
// the session runs with the same environment, working directory and user.
func sessionProcessSpec(
	ctx context.Context, task containerd.Task, opts SessionOptions,
) (*specs.Process, error) {
	spec, err := task.Spec(ctx)
	if err != nil {
		return nil, err
	}

	process := *spec.Process
	process.Args = opts.Args
	process.Terminal = true
	process.Env = append([]string(nil), spec.Process.Env...)

	process.Env = append(process.Env, "TERM="+sessionTerm)

	return &process, nil
}

// startProcess subscribes to the exit status before starting the process so
// a process that exits immediately isn't missed.
func startProcess(ctx context.Context, process containerd.Process) (*Session, error) {
	// The exit status must outlive the request context of the caller.
	exitC, err := process.Wait(context.WithoutCancel(ctx))
	if err != nil {
		return nil, err
	}

	if err := process.Start(ctx); err != nil {
		return nil, fmt.Errorf("could not start session: %w", err)
	}

	return &Session{process: process, exitC: exitC}, nil
}

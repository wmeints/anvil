// Package daemon implements the anvil daemon that manages sandboxes on the local
// containerd runtime.
package daemon

import (
	"errors"
	"fmt"
	"io/fs"
	"log/slog"
	"net"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"

	containerd "github.com/containerd/containerd/v2/client"
	"github.com/wmeints/anvil/api/v1alpha1"
	"github.com/wmeints/anvil/internal/paths"
	"google.golang.org/grpc"
)

// ErrStaleSocket is returned when a socket file left behind by a previous run
// can't be removed.
var ErrStaleSocket = errors.New("failed to remove old socket file")

// ErrSocketDirectory is returned when the directory for the daemon socket can't
// be created.
var ErrSocketDirectory = errors.New("failed to create socket directory")

// ErrListen is returned when the daemon can't listen on its socket.
var ErrListen = errors.New("failed to listen on socket")

// ErrContainerRuntime is returned when the daemon can't connect to containerd.
var ErrContainerRuntime = errors.New("failed to connect to containerd")

// ErrServe is returned when the daemon stops serving the control API because
// of an error.
var ErrServe = errors.New("failed to serve the control API")

func createListener() (net.Listener, error) {
	sock, err := paths.DaemonSocket()

	if err != nil {
		return nil, err
	}

	// Make sure the socket is private to the current user.
	if err := os.MkdirAll(filepath.Dir(sock), 0o700); err != nil {
		return nil, fmt.Errorf("%w: %s %w", ErrSocketDirectory, filepath.Dir(sock), err)
	}

	// Clean up old socket files before attempting to listen on it.
	// Notify the user if the old socket can't be removed.
	err = os.Remove(sock)
	if err != nil && !errors.Is(err, fs.ErrNotExist) {
		return nil, fmt.Errorf(
			"%w: %s (please manually remove it) %w", ErrStaleSocket, sock, err)
	}

	lis, err := net.Listen("unix", sock)
	if err != nil {
		return nil, fmt.Errorf("%w: %s %w", ErrListen, sock, err)
	}

	return lis, nil
}

func newContainerClient() (*containerd.Client, error) {
	slog.Info("connecting to rootless containerd socket")

	sock := paths.ContainerRuntimeSocket()

	client, err := containerd.New(sock, containerd.WithDefaultNamespace("anvil"))
	if err != nil {
		return nil, fmt.Errorf(
			"%w: %s (check that rootless containerd is running) %w",
			ErrContainerRuntime, sock, err)
	}

	return client, nil
}

// Run connects to containerd and serves the anvil control API until the daemon
// is shut down.
func Run() error {
	containerClient, err := newContainerClient()
	if err != nil {
		return err
	}

	defer func() {
		_ = containerClient.Close()
	}()

	listener, err := createListener()
	if err != nil {
		return err
	}

	defer func() {
		_ = listener.Close()
	}()

	grpcServer := grpc.NewServer(grpc.WaitForHandlers(true))
	anvilServer, err := newAnvilService(containerClient)
	if err != nil {
		return err
	}

	v1alpha1.RegisterAnvilServiceServer(grpcServer, anvilServer)

	go func() {
		go func() {
			sig := make(chan os.Signal, 1)
			signal.Notify(sig, syscall.SIGINT, syscall.SIGTERM)
			<-sig
			slog.Info("shutting down server")
			grpcServer.Stop()
		}()
	}()

	slog.Info("listening for requests")

	if err := grpcServer.Serve(listener); err != nil {
		return fmt.Errorf("%w: %w", ErrServe, err)
	}

	return nil
}

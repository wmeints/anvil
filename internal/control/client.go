// Package control provides the client the CLI uses to talk to the anvil daemon.
package control

import (
	"context"
	"fmt"
	"time"

	"github.com/wmeints/anvil/api/v1alpha1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/protobuf/types/known/durationpb"
)

// Client provides an interface to communicate with the daemon process
type Client struct {
	anvilClient v1alpha1.AnvilServiceClient
	serverConn  *grpc.ClientConn
}

// Close shuts down the server connection.
func (client *Client) Close() error {
	return client.serverConn.Close()
}

// ListSandboxes returns a list of sandboxes.
func (client *Client) ListSandboxes(
	ctx context.Context,
) ([]*v1alpha1.SandboxSummary, error) {
	resp, err := client.anvilClient.ListSandboxes(ctx, &v1alpha1.ListSandboxesRequest{})
	if err != nil {
		return nil, err
	}

	return resp.Sandboxes, nil
}

// CreateSandbox creates a new sandbox based on the provided settings
func (client *Client) CreateSandbox(
	ctx context.Context, name string, image string,
) error {
	_, err := client.anvilClient.CreateSandbox(ctx, &v1alpha1.CreateSandboxRequest{
		Name:  name,
		Image: image,
	})

	if err != nil {
		return err
	}

	return nil
}

// RemoveSandbox removes an existing sandbox
func (client *Client) RemoveSandbox(ctx context.Context, name string) error {
	_, err := client.anvilClient.RemoveSandbox(ctx, &v1alpha1.RemoveSandboxRequest{
		Name: name,
	})

	if err != nil {
		return err
	}

	return nil
}

// StopSandbox stops the VM of an existing sandbox and keeps its disk. The
// processes in the sandbox get the timeout to terminate before they're killed.
func (client *Client) StopSandbox(
	ctx context.Context, name string, timeout time.Duration,
) error {
	_, err := client.anvilClient.StopSandbox(ctx, &v1alpha1.StopSandboxRequest{
		Name:    name,
		Timeout: durationpb.New(timeout),
	})

	return err
}

// StartSandbox boots the VM of an existing sandbox that isn't running.
func (client *Client) StartSandbox(ctx context.Context, name string) error {
	_, err := client.anvilClient.StartSandbox(ctx, &v1alpha1.StartSandboxRequest{
		Name: name,
	})

	return err
}

// New creates a new control client.
// The address must point to the file containing the daemon socket.
func New(addr string) (*Client, error) {
	if addr == "" {
		return nil, fmt.Errorf("invalid socket address")
	}

	credentials := grpc.WithTransportCredentials(insecure.NewCredentials())

	conn, err := grpc.NewClient(addr, credentials)
	if err != nil {
		return nil, err
	}

	anvilClient := v1alpha1.NewAnvilServiceClient(conn)

	return &Client{
		anvilClient: anvilClient,
		serverConn:  conn,
	}, nil
}

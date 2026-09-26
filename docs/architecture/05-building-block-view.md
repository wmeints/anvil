# Building block view

## Level 1 - Building blocks

This application has two main components to it:

1. `anvild` - This runs the main user deamon process managing the lifecycle of the sandboxes.
2. `anvil` - This runs the CLI connecting to the sandboxes through the daemon. 

Both components are connected over a unix socket with peer credentials to ensure 
that only the `anvil` CLI process can talk to the daemon API. We use gRPC as the
protocol running over the socket.

## Level 2 - CLI

The purpose of the CLI is to execute tasks against the daemon. It has the 
following structure:

TODO: describe the component structure

## Level 2 - Daemon

The purpose of the daemon is to manage the sandboxes and sessions running 
against the sandboxes. It has the following structure:

TODO: describe the component structure
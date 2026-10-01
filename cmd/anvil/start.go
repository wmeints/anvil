package main

import "context"

// StartCmd boots the VM of a stopped sandbox without opening a session.
type StartCmd struct {
	Name string `arg:"" help:"name of sandbox to start"`
}

// Run executes the start command.
func (cmd *StartCmd) Run(ctx *Context) error {
	return ctx.client.StartSandbox(context.Background(), cmd.Name)
}

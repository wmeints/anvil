package main

import "context"

type RmCmd struct {
	Name string `arg:"" help:"name of sandbox to remove"`
}

func (cmd *RmCmd) Run(ctx *Context) error {
	runCtx := context.Background()
	return ctx.client.RemoveSandbox(runCtx, cmd.Name)
}

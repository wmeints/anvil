package main

import "context"

type CreateCmd struct {
	Name  string `short:"n" long:"name" required:"yes"`
	Image string `short:"i" long:"image" default:"ghcr.io/wmeints/anvil-base:latest"`
}

func (cmd *CreateCmd) Run(ctx *Context) error {
	runCtx := context.Background()
	return ctx.client.CreateSandbox(runCtx, cmd.Name, cmd.Image)
}

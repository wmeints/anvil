package main

import "context"

type CreateCmd struct {
	Name  string `short:"n" long:"name" required:"yes"`
	Image string `short:"i" long:"image" required:"yes" default:"ubuntu:26.04"`
}

func (cmd *CreateCmd) Run(ctx *Context) error {
	runCtx := context.Background()
	return ctx.client.CreateSandbox(runCtx, cmd.Name, cmd.Image)
}

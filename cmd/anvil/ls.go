package main

import (
	"context"
	"fmt"
)

type LsCmd struct{}

func (cmd *LsCmd) Run(ctx *Context) error {
	runCtx := context.Background()
	sandboxes, err := ctx.client.ListSandboxes(runCtx)
	if err != nil {
		return err
	}

	for _, sandbox := range sandboxes {
		_, _ = fmt.Fprintln(ctx.stdout, sandbox.Name)
	}

	return nil
}

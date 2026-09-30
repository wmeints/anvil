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
		fmt.Println(sandbox.Name)
	}

	return nil
}

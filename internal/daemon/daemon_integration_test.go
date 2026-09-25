//go:build integration

package daemon

import (
	"context"
	"testing"
)

func TestStartStop(t *testing.T) {
	ctx := context.Background()
	t.Setenv("XDG_DATA_HOME", t.TempDir())
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())

	d, err := Start(ctx, Options{NerdboxDir: t.TempDir()})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := d.Client().Version(ctx); err != nil {
		t.Fatal(err)
	}
	if err := d.Stop(); err != nil {
		t.Fatal(err)
	}
}

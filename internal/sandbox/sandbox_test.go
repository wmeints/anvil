package sandbox

import (
	"errors"
	"testing"
)

func TestNewSandbox(t *testing.T) {
	for _, name := range []string{"agent", "agent1", "agent-1", "my-agent-2b"} {
		t.Run(name, func(t *testing.T) {
			assertValidSandbox(t, name)
		})
	}
}

func assertValidSandbox(t *testing.T, name string) {
	t.Helper()

	sb, err := NewSandbox(name, "ubuntu:26.04")
	if err != nil {
		t.Fatalf("NewSandbox(%q): %v", name, err)
	}

	if sb.Name != name || sb.Spec.Image != "ubuntu:26.04" {
		t.Errorf("unexpected sandbox: %+v", sb)
	}
}

func TestNewSandboxRejectsInvalidInput(t *testing.T) {
	tests := []struct {
		name, sandbox, image string
		want                 error
	}{
		{"empty name", "", "ubuntu:26.04", ErrInvalidName},
		{"upper case", "Agent", "ubuntu:26.04", ErrInvalidName},
		{"leading digit", "1agent", "ubuntu:26.04", ErrInvalidName},
		{"trailing dash", "agent-", "ubuntu:26.04", ErrInvalidName},
		{"double dash", "my--agent", "ubuntu:26.04", ErrInvalidName},
		{"empty image", "agent", "", ErrInvalidImage},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if _, err := NewSandbox(tt.sandbox, tt.image); !errors.Is(err, tt.want) {
				t.Errorf("error = %v, want %v", err, tt.want)
			}
		})
	}
}

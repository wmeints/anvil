package sandbox

import (
	"context"
	"errors"
	"reflect"
	"slices"
	"strings"
	"testing"

	"github.com/containerd/containerd/v2/core/containers"
	"github.com/containerd/containerd/v2/pkg/cap"
	"github.com/containerd/containerd/v2/pkg/namespaces"
	"github.com/containerd/containerd/v2/pkg/oci"
	ocispec "github.com/opencontainers/image-spec/specs-go/v1"
	"github.com/opencontainers/runtime-spec/specs-go"
)

func TestNewSandbox(t *testing.T) {
	names := []string{
		"agent", "agent1", "agent-1", "my-agent-2b", strings.Repeat("a", 63),
	}

	for _, name := range names {
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
		{"too long", strings.Repeat("a", 64), "ubuntu:26.04", ErrInvalidName},
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

func TestParseImageUser(t *testing.T) {
	tests := []struct {
		user     string
		uid, gid uint32
	}{
		{"1000", 1000, 1000},
		{"1000:1001", 1000, 1001},
	}

	for _, tt := range tests {
		t.Run(tt.user, func(t *testing.T) {
			assertImageUser(t, tt.user, tt.uid, tt.gid)
		})
	}
}

func assertImageUser(t *testing.T, user string, wantUID, wantGID uint32) {
	t.Helper()

	uid, gid, err := parseImageUser(user)
	if err != nil {
		t.Fatal(err)
	}

	if uid != wantUID || gid != wantGID {
		t.Errorf("user = %d:%d, want %d:%d", uid, gid, wantUID, wantGID)
	}
}

func TestParseImageUserRejectsInvalidUser(t *testing.T) {
	tests := []struct{ user, want string }{
		{"", "has no user"},
		{"agent", `user "agent" is not numeric`},
		{"agent:1000", `user "agent:1000" is not numeric`},
		{"1000:agent", `user "1000:agent" is not numeric`},
		{"agent:99999999999", `user "agent:99999999999" is not numeric`},
		{"0", `user "0" is root`},
		{"0:0", `user "0:0" is root`},
		{"1000:0", `user "1000:0" is root`},
		{"0:1000", `user "0:1000" is root`},
		{"99999999999", `user "99999999999" is out of range`},
	}

	for _, tt := range tests {
		t.Run(tt.user, func(t *testing.T) {
			assertInvalidImageUser(t, tt.user, tt.want)
		})
	}
}

func assertInvalidImageUser(t *testing.T, user, want string) {
	t.Helper()

	_, _, err := parseImageUser(user)
	if !errors.Is(err, ErrInvalidImageUser) {
		t.Fatalf("error = %v, want %v", err, ErrInvalidImageUser)
	}

	if !strings.Contains(err.Error(), want) {
		t.Errorf("error = %q, want it to contain %q", err, want)
	}
}

func TestProcessArgs(t *testing.T) {
	tests := []struct {
		name   string
		config ocispec.ImageConfig
		want   []string
	}{
		{"no entrypoint", ocispec.ImageConfig{}, []string{"sleep", "infinity"}},
		{
			"entrypoint",
			ocispec.ImageConfig{Entrypoint: []string{"/e"}},
			[]string{"/e", "sleep", "infinity"},
		},
		{
			"ignores cmd",
			ocispec.ImageConfig{Entrypoint: []string{"/e"}, Cmd: []string{"bash"}},
			[]string{"/e", "sleep", "infinity"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := processArgs(tt.config); !slices.Equal(got, tt.want) {
				t.Errorf("args = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestWithVMPrivileges(t *testing.T) {
	ctx := namespaces.WithNamespace(context.Background(), "anvil-test")

	spec, err := oci.GenerateSpec(ctx, nil, &containers.Container{ID: "test"},
		withVMPrivileges)
	if err != nil {
		t.Fatal(err)
	}

	assertAllCapabilities(t, spec.Process.Capabilities)

	if spec.Process.NoNewPrivileges {
		t.Error("expected NoNewPrivileges to be false")
	}

	if len(spec.Linux.MaskedPaths) > 0 || len(spec.Linux.ReadonlyPaths) > 0 {
		t.Errorf("expected no masked or read-only paths, got %q and %q",
			spec.Linux.MaskedPaths, spec.Linux.ReadonlyPaths)
	}

	assertWritableMounts(t, spec.Mounts)
	assertCgroupMount(t, spec.Mounts)

	if spec.Linux.Seccomp != nil {
		t.Errorf("expected no seccomp profile, got %+v", spec.Linux.Seccomp)
	}
}

func assertAllCapabilities(t *testing.T, caps *specs.LinuxCapabilities) {
	t.Helper()

	known := cap.Known()
	for _, set := range [][]string{caps.Bounding, caps.Effective, caps.Permitted} {
		if !slices.Equal(set, known) {
			t.Errorf("capabilities = %q, want all known %q", set, known)
		}
	}
}

// assertWritableMounts verifies that the sysfs and cgroup mounts aren't
// read-only.
func assertWritableMounts(t *testing.T, mounts []specs.Mount) {
	t.Helper()

	for _, m := range mounts {
		writable := m.Type != "sysfs" && m.Type != "cgroup" ||
			!slices.Contains(m.Options, "ro")
		if !writable {
			t.Errorf("expected %s to be writable, got options %q",
				m.Destination, m.Options)
		}
	}
}

// assertCgroupMount verifies that the spec mounts the cgroup filesystem, which
// dockerd needs.
func assertCgroupMount(t *testing.T, mounts []specs.Mount) {
	t.Helper()

	cgroup := slices.ContainsFunc(mounts, func(m specs.Mount) bool {
		return m.Destination == "/sys/fs/cgroup" && m.Type == "cgroup"
	})
	if !cgroup {
		t.Errorf("expected a cgroup mount at /sys/fs/cgroup, got %+v", mounts)
	}
}

func TestVolumeMounts(t *testing.T) {
	config := ocispec.ImageConfig{Volumes: map[string]struct{}{
		"/var/lib/docker": {},
		"/data":           {},
	}}

	got := volumeMounts(config)
	options := []string{"rw", "nosuid", "nodev"}

	want := []specs.Mount{
		{Destination: "/data", Type: "tmpfs", Source: "tmpfs", Options: options},
		{
			Destination: "/var/lib/docker", Type: "tmpfs", Source: "tmpfs",
			Options: options,
		},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("mounts = %+v, want %+v", got, want)
	}
}

func TestSandboxSpecOpts(t *testing.T) {
	config := ocispec.ImageConfig{
		User:       "1000:1001",
		Env:        []string{"A=1"},
		Entrypoint: []string{"/e"},
		Cmd:        []string{"bash"},
		Volumes:    map[string]struct{}{"/data": {}},
	}

	spec := generateSandboxSpec(t, "demo", config)

	if spec.Hostname != "demo" {
		t.Errorf("hostname = %q, want the sandbox name %q", spec.Hostname, "demo")
	}

	if user := spec.Process.User; user.UID != 1000 || user.GID != 1001 {
		t.Errorf("user = %d:%d, want 1000:1001", user.UID, user.GID)
	}

	assertImageProcess(t, spec.Process)
	assertVolumeMount(t, spec.Mounts, "/data")
}

// assertImageProcess verifies the args, environment and working directory that
// TestSandboxSpecOpts derives from its image config.
func assertImageProcess(t *testing.T, process *specs.Process) {
	t.Helper()

	want := []string{"/e", "sleep", "infinity"}
	if !slices.Equal(process.Args, want) {
		t.Errorf("args = %q, want %q", process.Args, want)
	}

	if !slices.Contains(process.Env, "A=1") || process.Cwd != "/" {
		t.Errorf("env = %q and cwd = %q, want A=1 and /", process.Env, process.Cwd)
	}
}

func TestSandboxSpecOptsUsesWorkingDir(t *testing.T) {
	config := ocispec.ImageConfig{User: "1000", WorkingDir: "/home/agent"}

	if cwd := generateSandboxSpec(t, "demo", config).Process.Cwd; cwd != "/home/agent" {
		t.Errorf("cwd = %q, want /home/agent", cwd)
	}
}

// generateSandboxSpec generates the spec of a sandbox from the image config.
func generateSandboxSpec(
	t *testing.T, name string, config ocispec.ImageConfig,
) *oci.Spec {
	t.Helper()

	opts, err := sandboxSpecOpts(name, config)
	if err != nil {
		t.Fatal(err)
	}

	ctx := namespaces.WithNamespace(context.Background(), "anvil-test")

	spec, err := oci.GenerateSpec(ctx, nil, &containers.Container{ID: name}, opts)
	if err != nil {
		t.Fatal(err)
	}

	return spec
}

func TestSandboxSpecOptsDoesNotShareMountOptions(t *testing.T) {
	config := ocispec.ImageConfig{User: "1000", Volumes: map[string]struct{}{"/a": {}}}

	first := generateSandboxSpec(t, "one", config)
	for i := range first.Mounts {
		first.Mounts[i].Options = append(first.Mounts[i].Options[:0], "changed")
	}

	for _, m := range generateSandboxSpec(t, "two", config).Mounts {
		if slices.Contains(m.Options, "changed") {
			t.Fatalf("mount %s shares its options with another spec", m.Destination)
		}
	}
}

func TestSandboxSpecOptsRejectsInvalidUser(t *testing.T) {
	_, err := sandboxSpecOpts("demo", ocispec.ImageConfig{})
	if !errors.Is(err, ErrInvalidImageUser) {
		t.Errorf("error = %v, want %v", err, ErrInvalidImageUser)
	}
}

func assertVolumeMount(t *testing.T, mounts []specs.Mount, dest string) {
	t.Helper()

	found := slices.ContainsFunc(mounts, func(m specs.Mount) bool {
		return m.Destination == dest && m.Type == "tmpfs"
	})
	if !found {
		t.Errorf("expected a tmpfs at %s, got %+v", dest, mounts)
	}
}

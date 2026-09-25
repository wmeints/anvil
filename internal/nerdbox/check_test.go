package nerdbox

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestCheck(t *testing.T) {
	arch := kernelArch()
	complete := []string{
		"containerd-shim-nerdbox-v1",
		"libkrun.so",
		"nerdbox-kernel-" + arch,
		"nerdbox-rootfs.erofs",
	}

	tests := []struct {
		name    string
		files   []string
		noKVM   bool
		noErofs bool
		noMkfs  bool
		wantErr []string
	}{
		{name: "complete", files: complete},
		{
			name:  "arch specific variants",
			files: []string{"containerd-shim-nerdbox-v1", "libkrun-" + arch + ".so", "nerdbox-kernel-" + arch, "nerdbox-rootfs-" + arch + ".erofs"},
		},
		{name: "empty dir", wantErr: []string{"containerd-shim-nerdbox-v1", "libkrun.so", "nerdbox-kernel-", "nerdbox-rootfs.erofs"}},
		{name: "missing kernel", files: []string{"containerd-shim-nerdbox-v1", "libkrun.so", "nerdbox-rootfs.erofs"}, wantErr: []string{"nerdbox-kernel-" + arch}},
		{name: "no kvm", files: complete, noKVM: true, wantErr: []string{"cannot open"}},
		{name: "erofs module not loaded", files: complete, noErofs: true, wantErr: []string{"modprobe erofs"}},
		{name: "no mkfs.erofs", files: complete, noMkfs: true, wantErr: []string{"erofs-utils"}},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			dir := t.TempDir()
			for _, f := range tt.files {
				writeFile(t, filepath.Join(dir, f), "", 0o644)
			}

			kvm := filepath.Join(dir, "kvm")
			if !tt.noKVM {
				writeFile(t, kvm, "", 0o666)
			}

			filesystems := filepath.Join(dir, "filesystems")
			content := "nodev\ttmpfs\n\text4\n"
			if !tt.noErofs {
				content += "\terofs\n"
			}
			writeFile(t, filesystems, content, 0o644)

			bin := t.TempDir()
			if !tt.noMkfs {
				writeFile(t, filepath.Join(bin, "mkfs.erofs"), "#!/bin/sh\n", 0o755)
			}
			t.Setenv("PATH", bin)

			err := check(dir, kvm, filesystems)
			if len(tt.wantErr) == 0 {
				if err != nil {
					t.Fatalf("unexpected error: %v", err)
				}
				return
			}
			if err == nil {
				t.Fatal("expected an error")
			}
			for _, want := range tt.wantErr {
				if !strings.Contains(err.Error(), want) {
					t.Errorf("error %q does not mention %q", err, want)
				}
			}
		})
	}
}

func writeFile(t *testing.T, path, content string, mode os.FileMode) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), mode); err != nil {
		t.Fatal(err)
	}
}

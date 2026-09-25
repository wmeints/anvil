// Package nerdbox verifies that the host has what nerdbox needs to boot a microVM.
package nerdbox

import (
	"bytes"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
)

// RuntimeName is the containerd runtime name of the nerdbox shim.
const RuntimeName = "io.containerd.nerdbox.v1"

// Check verifies that /dev/kvm is usable, that the host can build erofs
// images for the erofs snapshotter, and that dir contains the nerdbox shim,
// libkrun, guest kernel and guest rootfs. The returned error lists every
// problem found.
func Check(dir string) error {
	return check(dir, "/dev/kvm", "/proc/filesystems")
}

func check(dir, kvmPath, filesystemsPath string) error {
	var problems []string

	if _, err := exec.LookPath("mkfs.erofs"); err != nil {
		problems = append(problems, "mkfs.erofs not found, install erofs-utils")
	}
	// containerd only enables its erofs snapshotter when the kernel knows erofs.
	if fs, err := os.ReadFile(filesystemsPath); err != nil || !bytes.Contains(fs, []byte("\terofs\n")) {
		problems = append(problems, "erofs kernel module not loaded, run: sudo modprobe erofs")
	}

	if f, err := os.OpenFile(kvmPath, os.O_RDWR, 0); err != nil {
		problems = append(problems, fmt.Sprintf("cannot open %s: %v", kvmPath, err))
	} else {
		_ = f.Close()
	}

	arch := kernelArch()
	artifacts := [][]string{
		{"containerd-shim-nerdbox-v1"},
		{"libkrun.so", "libkrun-" + arch + ".so"},
		{"nerdbox-kernel-" + arch},
		{"nerdbox-rootfs.erofs", "nerdbox-rootfs-" + arch + ".erofs"},
	}
	for _, names := range artifacts {
		if !anyExists(dir, names) {
			problems = append(problems, fmt.Sprintf("missing %s in %s", strings.Join(names, " or "), dir))
		}
	}

	if len(problems) > 0 {
		return errors.New("nerdbox is not ready:\n  " + strings.Join(problems, "\n  "))
	}
	return nil
}

func anyExists(dir string, names []string) bool {
	for _, name := range names {
		if _, err := os.Stat(filepath.Join(dir, name)); err == nil {
			return true
		}
	}
	return false
}

// kernelArch maps the Go architecture to the naming used by nerdbox artifacts.
func kernelArch() string {
	if runtime.GOARCH == "amd64" {
		return "x86_64"
	}
	return runtime.GOARCH
}

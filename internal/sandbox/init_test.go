package sandbox

import (
	"bytes"
	"io/fs"
	"os"
	"path/filepath"
	"testing"
)

func TestInstallInit(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "init")

	if err := installInit(dir); err != nil {
		t.Fatal(err)
	}

	assertInitInstalled(t, dir)

	info, err := os.Stat(dir)
	if err != nil {
		t.Fatal(err)
	}

	if mode := info.Mode().Perm(); mode != 0o700 {
		t.Errorf("init dir mode = %v, want %v", mode, os.FileMode(0o700))
	}
}

func TestInstallInitKeepsIdenticalFile(t *testing.T) {
	dir := t.TempDir()

	if err := installInit(dir); err != nil {
		t.Fatal(err)
	}

	before, err := os.Stat(filepath.Join(dir, "tini"))
	if err != nil {
		t.Fatal(err)
	}

	if err := installInit(dir); err != nil {
		t.Fatal(err)
	}

	after, err := os.Stat(filepath.Join(dir, "tini"))
	if err != nil {
		t.Fatal(err)
	}

	if !os.SameFile(before, after) {
		t.Error("expected an identical init binary to be kept, it was replaced")
	}
}

func TestInstallInitReplacesChangedFile(t *testing.T) {
	dir := t.TempDir()

	err := os.WriteFile(filepath.Join(dir, "tini"), []byte("stale"), 0o600)
	if err != nil {
		t.Fatal(err)
	}

	if err := installInit(dir); err != nil {
		t.Fatal(err)
	}

	assertInitInstalled(t, dir)
}

func TestInstallInitFailsForUnwritableDir(t *testing.T) {
	// A regular file in the path makes the directory impossible to create.
	file := filepath.Join(t.TempDir(), "file")
	if err := os.WriteFile(file, nil, 0o600); err != nil {
		t.Fatal(err)
	}

	if err := installInit(filepath.Join(file, "init")); err == nil {
		t.Error("expected installing the init to fail")
	}
}

func TestInstallInitCleansUpAfterFailedWrite(t *testing.T) {
	dir := t.TempDir()

	// A non-empty directory in place of the init can't be replaced by a rename.
	blocker := filepath.Join(dir, "tini", "x")
	if err := os.MkdirAll(blocker, 0o700); err != nil {
		t.Fatal(err)
	}

	if err := installInit(dir); err == nil {
		t.Fatal("expected installing the init to fail")
	}

	temps, err := filepath.Glob(filepath.Join(dir, ".tini-*"))
	if err != nil {
		t.Fatal(err)
	}

	if len(temps) != 0 {
		t.Errorf("expected no temporary files to be left, found %v", temps)
	}
}

func assertInitInstalled(t *testing.T, dir string) {
	t.Helper()

	data, err := fs.ReadFile(os.DirFS(dir), "tini")
	if err != nil {
		t.Fatal(err)
	}

	if !bytes.Equal(data, tini) {
		t.Error("installed init differs from the embedded tini binary")
	}

	assertOnlyInit(t, dir)

	info, err := os.Stat(filepath.Join(dir, "tini"))
	if err != nil {
		t.Fatal(err)
	}

	if mode := info.Mode().Perm(); mode != 0o755 {
		t.Errorf("init mode = %v, want %v", mode, os.FileMode(0o755))
	}
}

// assertOnlyInit verifies that no temporary files are left next to the init.
func assertOnlyInit(t *testing.T, dir string) {
	t.Helper()

	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}

	if len(entries) != 1 {
		t.Errorf("expected only the init in %s, found %d entries", dir, len(entries))
	}
}

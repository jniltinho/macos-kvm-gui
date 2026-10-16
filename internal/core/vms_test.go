package core

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestVMsDatabase(t *testing.T) {
	cfg := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", cfg)
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	app := filepath.Join(cfg, "macos-kvm-gui")

	// Files from older versions are imported into the new database and kept, renamed.
	legacy := Defaults()
	legacy.RAMMiB = 12288
	SaveConfig(filepath.Join(app, "config.json"), legacy)
	fromDir := Defaults()
	fromDir.Name, fromDir.Version = "Old Tahoe", "tahoe"
	SaveConfig(filepath.Join(app, "vms", "old-tahoe.json"), fromDir)
	write(t, filepath.Join(app, "selected"), "old-tahoe\n")

	vms, err := ListVMs()
	if err != nil || len(vms) != 2 {
		t.Fatalf("import: %+v %v", vms, err)
	}
	if !exists(filepath.Join(app, "config.json.migrated")) || !exists(filepath.Join(app, "vms.migrated")) || exists(filepath.Join(app, "config.json")) {
		t.Fatal("the old files must be renamed, not deleted")
	}
	if s, _ := SelectedVM(vms); s.ID != "old-tahoe" {
		t.Fatalf("the old selection must carry over: %+v", s)
	}
	if e, ok := FindVM(vms, "macos sonoma"); !ok || e.Config.RAMMiB != 12288 {
		t.Fatalf("the legacy config.json must become a VM named after its version: %+v", vms)
	}

	// Create, refuse duplicates (case-insensitive) and bad settings.
	c := Defaults()
	c.Name, c.Workspace = "Lab", filepath.Join(t.TempDir(), "lab")
	os.MkdirAll(c.Workspace, 0o755)
	e, err := CreateVM(c)
	if err != nil || e.ID != "lab" {
		t.Fatalf("create: %+v %v", e, err)
	}
	var fe *FieldError
	dup := c
	dup.Name = "LAB"
	if _, err := CreateVM(dup); !errors.As(err, &fe) || fe.Field != "name" {
		t.Fatalf("duplicate names must be refused: %v", err)
	}
	bad := c
	bad.Name, bad.Cores, bad.VCPUs = "Bad", 4, 6
	if _, err := CreateVM(bad); !errors.As(err, &fe) || fe.Field != "vcpus" {
		t.Fatalf("invalid settings must be refused: %v", err)
	}

	// Save updates the row and regenerates the workspace snapshot.
	c.RAMMiB = 16384
	if err := SaveVM(e, c); err != nil {
		t.Fatal(err)
	}
	vms, _ = ListVMs()
	if got, _ := FindVM(vms, "lab"); got.Config.RAMMiB != 16384 {
		t.Fatalf("save: %+v", got)
	}
	var snap struct{ Config Config }
	b, err := os.ReadFile(filepath.Join(c.Workspace, snapshotFile))
	if err != nil || json.Unmarshal(b, &snap) != nil || snap.Config.RAMMiB != 16384 || !strings.Contains(string(b), "Generated from") {
		t.Fatalf("snapshot: %v %s", err, b)
	}
	if out, _ := ExportVM(e); !strings.Contains(string(out), `"name": "Lab"`) {
		t.Fatalf("export: %s", out)
	}

	// Select, then delete with purge: only the app's files go, the user's stay.
	if err := SelectVM(e.ID); err != nil {
		t.Fatal(err)
	}
	w := NewWorkspace(c.Workspace)
	if err := w.PrepareBoot(c, false); err != nil {
		t.Fatal(err)
	}
	write(t, w.Path(DiskFile), "disk")
	write(t, w.Path("notes.txt"), "the user's own file")
	if _, err := DeleteVM(e, true); err != nil {
		t.Fatal(err)
	}
	if exists(w.Path(DiskFile)) || exists(w.Path(BootDir)) {
		t.Fatal("purge must delete the app's files")
	}
	if b, _ := os.ReadFile(w.Path("notes.txt")); string(b) != "the user's own file" {
		t.Fatal("files the app did not create must survive")
	}
	if vms, _ := ListVMs(); len(vms) != 2 {
		t.Fatalf("two VMs must remain: %+v", vms)
	}
}

func TestMoveVM(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	old := t.TempDir()
	c := Defaults()
	c.Name, c.Workspace = "Mover", old
	w := NewWorkspace(old)
	if err := w.PrepareBoot(c, false); err != nil {
		t.Fatal(err)
	}
	os.WriteFile(w.Path(DiskFile), []byte("disk"), 0o644)
	os.WriteFile(w.Path("OpenCore-Boot.sh"), []byte("not ours"), 0o644)
	e, err := CreateVM(c)
	if err != nil {
		t.Fatal(err)
	}
	if err := SetWorkspaceRoot(t.TempDir()); err != nil {
		t.Fatal(err)
	}
	dst := VMFolder(e.Config.Name)
	got, err := MoveVM(e, dst)
	if err != nil {
		t.Fatal(err)
	}
	if b, _ := os.ReadFile(filepath.Join(dst, DiskFile)); string(b) != "disk" || !exists(filepath.Join(dst, VarsFile)) {
		t.Fatal("disk and NVRAM must move")
	}
	if exists(w.Path(DiskFile)) || !exists(w.Path("OpenCore-Boot.sh")) {
		t.Fatal("only the app's files may move")
	}
	if vms, _ := ListVMs(); len(vms) != 1 || vms[0].Config.Workspace != dst || got.Config.Workspace != dst {
		t.Fatalf("the VM must point to the new folder: %+v", vms)
	}
	full := t.TempDir()
	os.WriteFile(filepath.Join(full, DiskFile), []byte("other"), 0o644)
	if _, err := MoveVM(got, full); err == nil || !exists(filepath.Join(dst, DiskFile)) {
		t.Fatal("a non-empty folder must be refused and nothing moved")
	}
	if _, err := MoveVM(got, filepath.Join(dst, "inner")); err == nil {
		t.Fatal("moving into itself must be refused")
	}
}

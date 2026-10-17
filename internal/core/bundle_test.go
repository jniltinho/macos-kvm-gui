package core

import (
	"archive/tar"
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func TestBundleRoundTrip(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	c := testDefaults()
	c.Name, c.Workspace, c.GPU = "Lab", filepath.Join(t.TempDir(), "lab"), "0000:03:00.0"
	w := NewWorkspace(c.Workspace)
	if err := w.PrepareBoot(c, false); err != nil {
		t.Fatal(err)
	}
	qcow2(t, w.Path(DiskFile), "")
	write(t, w.Path(MediaFile), "media")
	e, err := CreateVM(c)
	if err != nil {
		t.Fatal(err)
	}

	dst := filepath.Join(t.TempDir(), "lab.tar")
	calls := 0
	if err := ExportBundle(context.Background(), e, dst, func(int64, int64) { calls++ }); err != nil {
		t.Fatal(err)
	}
	if calls == 0 {
		t.Error("export must report progress")
	}

	ws2 := filepath.Join(t.TempDir(), "copy")
	got, err := ImportBundle(context.Background(), dst, ws2, nil)
	if err != nil {
		t.Fatal(err)
	}
	if got.Config.Name != "Lab (imported)" || got.Config.Workspace != ws2 || got.Config.GPU != "" {
		t.Fatalf("imported config: %+v", got.Config)
	}
	for _, f := range []string{DiskFile, VarsFile, identityFile, MediaFile} {
		a, _ := os.ReadFile(w.Path(f))
		b, err := os.ReadFile(filepath.Join(ws2, f))
		if err != nil || string(a) != string(b) {
			t.Errorf("%s did not round-trip: %v", f, err)
		}
	}
	if exists(filepath.Join(ws2, EFIDir)) {
		t.Error("OpenCore is regenerated at start, not shipped in the bundle")
	}

	// Importing into a folder that already has files is refused.
	if _, err := ImportBundle(context.Background(), dst, ws2, nil); err == nil {
		t.Fatal("a non-empty workspace must be refused")
	}
}

// qcow2 creates a small qcow2 disk, optionally on top of a backing file.
func qcow2(t *testing.T, path, backing string) {
	t.Helper()
	args := []string{"create", "-q", "-f", "qcow2"}
	if backing != "" {
		args = append(args, "-b", backing, "-F", "raw")
	}
	if out, err := exec.Command("qemu-img", append(args, path, "64M")...).CombinedOutput(); err != nil {
		t.Skipf("qemu-img unavailable: %v %s", err, out)
	}
}

func TestBundleRejectsDiskWithBackingFile(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	secret := filepath.Join(t.TempDir(), "host-file")
	write(t, secret, "host data")
	c := testDefaults()
	c.Name, c.Workspace = "Crafted", filepath.Join(t.TempDir(), "src")
	w := NewWorkspace(c.Workspace)
	if err := w.PrepareBoot(c, false); err != nil {
		t.Fatal(err)
	}
	qcow2(t, w.Path(DiskFile), secret)
	e, err := CreateVM(c)
	if err != nil {
		t.Fatal(err)
	}
	dst := filepath.Join(t.TempDir(), "crafted.tar")
	if err := ExportBundle(context.Background(), e, dst, nil); err != nil {
		t.Fatal(err)
	}
	ws := filepath.Join(t.TempDir(), "imported")
	if _, err := ImportBundle(context.Background(), dst, ws, nil); err == nil || !strings.Contains(err.Error(), "standalone") {
		t.Fatalf("a disk that references host files must be refused: %v", err)
	}
	if exists(filepath.Join(ws, DiskFile)) {
		t.Fatal("the refused disk must be removed")
	}
}

func TestBundleRejectsUnexpectedEntries(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	evil := filepath.Join(t.TempDir(), "evil.tar")
	f, _ := os.Create(evil)
	tw := tar.NewWriter(f)
	m := []byte(`{"format":1,"config":{"name":"Evil","workspace":"/tmp/x","version":"sonoma","ram_mib":8192,"cores":4,"vcpus":8,"ssh_port":2222}}`)
	tw.WriteHeader(&tar.Header{Name: bundleManifest, Mode: 0o644, Size: int64(len(m))})
	tw.Write(m)
	tw.WriteHeader(&tar.Header{Name: "../../escape.txt", Mode: 0o644, Size: 2})
	tw.Write([]byte("hi"))
	tw.Close()
	f.Close()

	parent := t.TempDir()
	ws := filepath.Join(parent, "ws")
	if _, err := ImportBundle(context.Background(), evil, ws, nil); err == nil || !strings.Contains(err.Error(), "unexpected bundle entry") {
		t.Fatalf("path traversal must be refused: %v", err)
	}
	if exists(filepath.Join(parent, "escape.txt")) || exists(ws) {
		t.Fatal("nothing may be left behind")
	}
	if vms, _ := ListVMs(); len(vms) != 0 {
		t.Fatal("no VM may be added")
	}
}

func TestImportSettings(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	c := testDefaults()
	c.Name = "From JSON"
	e, _ := CreateVM(c)
	b, _ := ExportVM(e)
	p := filepath.Join(t.TempDir(), "vm.json")
	os.WriteFile(p, b, 0o644)
	got, err := ImportSettings(p)
	if err != nil || got.Config.Name != "From JSON (imported)" {
		t.Fatalf("import settings: %+v %v", got, err)
	}
}

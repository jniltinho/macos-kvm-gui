package core

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// hostPorts makes port probing independent of the host: free reports what is free.
func hostPorts(t *testing.T, free func(int) bool) {
	old := portFree
	portFree = free
	t.Cleanup(func() { portFree = old })
}

func newVM(t *testing.T, name string, port int) VMEntry {
	t.Helper()
	c := Defaults()
	c.Name, c.SSHPort, c.Workspace = name, port, filepath.Join(t.TempDir(), Slug(name))
	e, err := CreateVM(c)
	if err != nil {
		t.Fatal(err)
	}
	return e
}

func TestNextSSHPort(t *testing.T) {
	all := func(int) bool { return true }
	for _, tc := range []struct {
		name  string
		taken []int
		probe func(int) bool
		want  int
	}{
		{"none", nil, all, 2222},
		{"2222 taken", []int{2222}, all, 2223},
		{"2222 and 2223 taken", []int{2222, 2223}, all, 2224},
		{"probe refuses 2223", []int{2222}, func(p int) bool { return p != 2223 }, 2224},
	} {
		t.Run(tc.name, func(t *testing.T) {
			t.Setenv("XDG_CONFIG_HOME", t.TempDir())
			hostPorts(t, all)
			for i, p := range tc.taken {
				newVM(t, string(rune('a'+i)), p)
			}
			if got, err := NextSSHPort("", tc.probe); err != nil || got != tc.want {
				t.Fatalf("NextSSHPort = %d, %v; want %d", got, err, tc.want)
			}
		})
	}
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	if _, err := NextSSHPort("", func(int) bool { return false }); err == nil || !strings.Contains(err.Error(), "no free SSH port from 2222 to 65535") {
		t.Fatalf("no free port must be an error: %v", err)
	}
	// A VM's own port does not count against it.
	hostPorts(t, all)
	a := newVM(t, "A", 0)
	if got, _ := NextSSHPort(a.ID, nil); got != 2222 {
		t.Fatalf("exceptID: %d", got)
	}
}

func TestSSHPortUsers(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	hostPorts(t, func(int) bool { return true })
	n := newVM(t, "Nilton", 2222)
	s := newVM(t, "sonoma-test", 2300)
	if names, err := SSHPortUsers(2222, s.ID); err != nil || len(names) != 1 || names[0] != "Nilton" {
		t.Fatalf("users: %v %v", names, err)
	}
	if names, _ := SSHPortUsers(2222, n.ID); len(names) != 0 {
		t.Fatalf("a VM is not its own user: %v", names)
	}
	if w := SSHPortWarning(2222, s.ID); w != "Also used by Nilton: they cannot run at the same time" {
		t.Fatalf("warning: %q", w)
	}
	if w := SSHPortWarning(2300, s.ID); w != "" {
		t.Fatalf("no warning for an unshared port: %q", w)
	}
}

func TestCreateAllocatesSSHPort(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	hostPorts(t, func(int) bool { return true })

	a := newVM(t, "A", 0)
	if a.Config.SSHPort != 2222 || a.Notice != "" {
		t.Fatalf("A: %d %q", a.Config.SSHPort, a.Notice)
	}
	b := newVM(t, "B", 2222)
	if b.Config.SSHPort != 2223 || b.Notice != "SSH port 2222 belongs to A; using 2223" {
		t.Fatalf("B: %d %q", b.Config.SSHPort, b.Notice)
	}

	// Imports keep their stored port unless another VM has it.
	imp := func(name string, port int) VMEntry {
		c := Defaults()
		c.Name, c.SSHPort, c.Workspace = name, port, filepath.Join(t.TempDir(), name)
		b, _ := ExportVM(VMEntry{Config: c})
		p := filepath.Join(t.TempDir(), name+".json")
		os.WriteFile(p, b, 0o644)
		e, err := ImportSettings(p)
		if err != nil {
			t.Fatal(err)
		}
		return e
	}
	if c := imp("C", 2222); c.Config.SSHPort != 2224 || !strings.Contains(c.Notice, "belongs to A") {
		t.Fatalf("C: %d %q", c.Config.SSHPort, c.Notice)
	}
	if d := imp("D", 2300); d.Config.SSHPort != 2300 || d.Notice != "" {
		t.Fatalf("D: %d %q", d.Config.SSHPort, d.Notice)
	}
	// The stored ports are the ones returned.
	vms, _ := ListVMs()
	got := map[string]int{}
	for _, e := range vms {
		got[e.Config.Name] = e.Config.SSHPort
	}
	if got["A"] != 2222 || got["B"] != 2223 || got["C"] != 2224 || got["D"] != 2300 {
		t.Fatalf("stored: %v", got)
	}

	// An explicit port held by another process (not a VM) is kept; out of range is refused.
	hostPorts(t, func(int) bool { return false })
	if e := newVM(t, "E", 2400); e.Config.SSHPort != 2400 {
		t.Fatalf("E: %d", e.Config.SSHPort)
	}
	bad := Defaults()
	bad.Name, bad.SSHPort = "Bad", 80
	if _, err := CreateVM(bad); err == nil || !strings.Contains(err.Error(), "use a port between 1024 and 65535") {
		t.Fatalf("port 80: %v", err)
	}

	// No free port: an error, and nothing is left behind.
	full := Defaults()
	full.Name = "Full"
	if _, err := CreateVM(full); err == nil || !strings.Contains(err.Error(), "no free SSH port") {
		t.Fatalf("exhaustion: %v", err)
	}
	if vms, _ := ListVMs(); len(vms) != 5 {
		t.Fatalf("no row may be added: %d VMs", len(vms))
	}
}

// Concurrent creates (the app and the CLI, or both apps) never pick the same port.
func TestConcurrentCreatesGetDistinctPorts(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	hostPorts(t, func(int) bool { return true })
	const n = 8
	ports := make(chan int, n)
	errs := make(chan error, n)
	for i := range n {
		go func() {
			c := Defaults()
			c.Name, c.Workspace = "vm"+string(rune('a'+i)), filepath.Join(t.TempDir(), "x")
			e, err := CreateVM(c)
			errs <- err
			ports <- e.Config.SSHPort
		}()
	}
	seen := map[int]bool{}
	for range n {
		if err := <-errs; err != nil {
			t.Fatal(err)
		}
		p := <-ports
		if seen[p] {
			t.Fatalf("port %d given twice", p)
		}
		seen[p] = true
	}
}

// An import that finds no free port removes the folder it unpacked.
func TestImportBundleNoFreePort(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	hostPorts(t, func(int) bool { return true })
	c := Defaults()
	c.Name, c.Workspace = "Src", filepath.Join(t.TempDir(), "src")
	w := NewWorkspace(c.Workspace)
	if err := w.PrepareBoot(testDefaults(), false); err != nil {
		t.Fatal(err)
	}
	qcow2(t, w.Path(DiskFile), "")
	e, err := CreateVM(c)
	if err != nil {
		t.Fatal(err)
	}
	dst := filepath.Join(t.TempDir(), "src.tar")
	if err := ExportBundle(context.Background(), e, dst, nil); err != nil {
		t.Fatal(err)
	}
	hostPorts(t, func(int) bool { return false })
	ws := filepath.Join(t.TempDir(), "copy")
	if _, err := ImportBundle(context.Background(), dst, ws, nil); err == nil || !strings.Contains(err.Error(), "no free SSH port") {
		t.Fatalf("import: %v", err)
	}
	if exists(ws) {
		t.Fatal("the unpacked folder must be removed")
	}
	if vms, _ := ListVMs(); len(vms) != 1 {
		t.Fatalf("no VM may be added: %d", len(vms))
	}
}

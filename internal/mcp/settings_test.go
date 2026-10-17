package mcp

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

// callOut calls a tool that must succeed and decodes its structured result.
func callOut(t *testing.T, cs *mcp.ClientSession, name string, args map[string]any) map[string]any {
	t.Helper()
	msg, isErr := call(t, cs, name, args)
	if isErr {
		t.Fatalf("%s %v: %s", name, args, msg)
	}
	var out map[string]any
	if err := json.Unmarshal([]byte(msg), &out); err != nil {
		t.Fatalf("%s: %v: %s", name, err, msg)
	}
	return out
}

func mustFail(t *testing.T, cs *mcp.ClientSession, name string, args map[string]any, want string) {
	t.Helper()
	msg, isErr := call(t, cs, name, args)
	if !isErr || !strings.Contains(msg, want) {
		t.Fatalf("%s %v: %q (error %v), want %q", name, args, msg, isErr, want)
	}
}

func createVM(t *testing.T, name string, port int) core.VMEntry {
	t.Helper()
	c := core.Defaults()
	c.Name, c.SSHPort, c.Workspace = name, port, filepath.Join(t.TempDir(), core.Slug(name))
	c.Networks = core.NewNetworks(c.Version)
	e, err := core.CreateVM(c)
	if err != nil {
		t.Fatal(err)
	}
	return e
}

// fakeQEMU runs a process that FindRunning takes for QEMU using e's disk.
func fakeQEMU(t *testing.T, e core.VMEntry) {
	t.Helper()
	sh, err := os.ReadFile("/bin/sh")
	if err != nil {
		t.Skip(err)
	}
	bin := filepath.Join(t.TempDir(), "qemu-system-x86_64")
	if err := os.WriteFile(bin, sh, 0o755); err != nil {
		t.Fatal(err)
	}
	dir, _ := e.Config.WorkspaceDir()
	cmd := exec.Command(bin, "-c", "sleep 30; :", "file="+filepath.Join(dir, core.DiskFile))
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { cmd.Process.Kill(); cmd.Wait() })
	for range 50 {
		if _, ok := core.FindRunning(e.Config); ok {
			return
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatal("the fake QEMU is not found")
}

func TestVMConfigGetSet(t *testing.T) {
	cs := session(t)
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	createVM(t, "Nilton", 2222)
	s := createVM(t, "sonoma-test", 2300)

	got := callOut(t, cs, "vm_config_get", map[string]any{"vm": "Nilton"})
	if got["ssh_port"] != 2222.0 || got["version"] != "sonoma" || got["ram_mib"] != 8192.0 || got["networks"] == nil {
		t.Fatalf("vm_config_get: %v", got)
	}

	// ssh-port and ram; a shared port saves, with the warning.
	got = callOut(t, cs, "vm_config_set", map[string]any{"vm": "sonoma-test", "ssh-port": 2222, "ram": 12288})
	if got["ssh_port"] != 2222.0 || got["ram_mib"] != 12288.0 {
		t.Fatalf("vm_config_set: %v", got)
	}
	if w, _ := got["warnings"].([]any); len(w) != 1 || w[0] != "Also used by Nilton: they cannot run at the same time" {
		t.Fatalf("warnings: %v", got["warnings"])
	}
	vms, _ := core.ListVMs()
	if e, _ := core.FindVM(vms, s.ID); e.Config.SSHPort != 2222 || e.Config.RAMMiB != 12288 {
		t.Fatalf("stored: %+v", e.Config)
	}
	if st := callOut(t, cs, "vm_status", map[string]any{"vm": "Nilton"}); st["warning"] != "Also used by sonoma-test: they cannot run at the same time" {
		t.Fatalf("vm_status warning: %v", st)
	}

	// Refusals change nothing.
	mustFail(t, cs, "vm_config_set", map[string]any{"vm": "sonoma-test", "root-patch": false}, "use the app or `config set`")
	mustFail(t, cs, "vm_config_set", map[string]any{"vm": "sonoma-test", "workspace": "/tmp/x"}, "use the app or `config set`")
	mustFail(t, cs, "vm_config_set", map[string]any{"vm": "sonoma-test", "ram": 100}, "minimum is 4096 MiB")
	mustFail(t, cs, "vm_config_set", map[string]any{"vm": "sonoma-test", "ssh-port": 80}, "use a port between 1024 and 65535")
	mustFail(t, cs, "vm_config_set", map[string]any{"vm": "sonoma-test", "graphics": "virtio"}, "vmware-svga or reims-vgpu")

	// The Workspace lock (a start in progress, or the other app) is held.
	dir, _ := s.Config.WorkspaceDir()
	unlock, err := core.NewWorkspace(dir).Lock()
	if err != nil {
		t.Fatal(err)
	}
	mustFail(t, cs, "vm_config_set", map[string]any{"vm": "sonoma-test", "ram": 16384}, "busy")
	unlock()

	// A running VM.
	fakeQEMU(t, s)
	mustFail(t, cs, "vm_config_set", map[string]any{"vm": "sonoma-test", "ram": 16384}, "shut the VM down first")
	vms, _ = core.ListVMs()
	if e, _ := core.FindVM(vms, s.ID); e.Config.RAMMiB != 12288 || e.Config.SSHPort != 2222 {
		t.Fatalf("a refused change must not be saved: %+v", e.Config)
	}
}

// An edit by the other app between our read and our save is not lost: the VM is reloaded in
// the transaction.
func TestVMConfigSetReloads(t *testing.T) {
	cs := session(t)
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	e := createVM(t, "Lab", 2400)
	c := e.Config
	c.VCPUs, c.Cores = 16, 8 // the other app's edit, saved after the agent read the VM
	if err := core.SaveVM(e, c); err != nil {
		t.Fatal(err)
	}
	got := callOut(t, cs, "vm_config_set", map[string]any{"vm": "Lab", "ram": 16384})
	if got["vcpus"] != 16.0 || got["ram_mib"] != 16384.0 {
		t.Fatalf("the other edit must survive: %v", got)
	}
}

func TestVMCreate(t *testing.T) {
	if _, err := exec.LookPath("qemu-img"); err != nil {
		t.Skip("qemu-img unavailable")
	}
	cs := session(t)
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	createVM(t, "Nilton", 2222)
	want, err := core.NextSSHPort("", nil)
	if err != nil || want == 2222 {
		t.Fatalf("next port %d %v", want, err)
	}
	folder := filepath.Join(t.TempDir(), "lab")
	got := callOut(t, cs, "vm_create", map[string]any{"name": "lab", "version": "sonoma", "folder": folder})
	if got["ssh_port"] != float64(want) || got["id"] != "lab" {
		t.Fatalf("vm_create: %v", got)
	}
	// What vm_start checks before starting QEMU: the folder, the disk and the boot files.
	for _, f := range []string{core.DiskFile, core.BootDir} {
		if _, err := os.Stat(filepath.Join(folder, f)); err != nil {
			t.Fatalf("%s: %v", f, err)
		}
	}
	vms, _ := core.ListVMs()
	if sel, _ := core.SelectedVM(vms); sel.ID != "lab" {
		t.Fatalf("the new VM must be selected: %s", sel.ID)
	}
	// An explicit port of another VM: the next free one, with a notice.
	got = callOut(t, cs, "vm_create", map[string]any{"name": "lab2", "ssh_port": 2222, "folder": filepath.Join(t.TempDir(), "lab2")})
	if got["ssh_port"] == 2222.0 || !strings.Contains(got["notice"].(string), "SSH port 2222 belongs to Nilton") {
		t.Fatalf("vm_create with a taken port: %v", got)
	}
	mustFail(t, cs, "vm_create", map[string]any{"name": "bad", "ssh_port": 80}, "use a port between 1024 and 65535")
	if vms, _ := core.ListVMs(); len(vms) != 3 {
		t.Fatalf("a refused create adds nothing: %d VMs", len(vms))
	}
}

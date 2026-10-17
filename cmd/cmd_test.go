package cmd

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

func run(t *testing.T, args ...string) (string, error) {
	t.Helper()
	var out bytes.Buffer
	rootCmd.SetOut(&out)
	rootCmd.SetErr(&out)
	rootCmd.SetArgs(args)
	err := rootCmd.Execute()
	return out.String(), err
}

func TestConfigCommands(t *testing.T) {
	cfg := filepath.Join(t.TempDir(), "config.json")
	if _, err := run(t, "--config", cfg, "config", "set", "version=sonoma", "ram=16384", "cores=8", "vcpus=16", "display=window"); err != nil {
		t.Fatal(err)
	}
	out, err := run(t, "--config", cfg, "config", "show")
	if err != nil || !strings.Contains(out, `"ram_mib": 16384`) || !strings.Contains(out, `"display": "window"`) {
		t.Fatalf("show: %v\n%s", err, out)
	}
	if _, err := run(t, "--config", cfg, "config", "set", "vcpus=6"); err == nil {
		t.Fatal("6 vCPUs on 8 cores must be refused")
	}
	if _, err := run(t, "--config", cfg, "config", "set", "nope=1"); err == nil {
		t.Fatal("unknown keys must be refused")
	}
	if out, _ := run(t, "--config", cfg, "config", "show"); !strings.Contains(out, `"graphics": "vmware-svga"`) {
		t.Fatalf("show must print the default graphics:\n%s", out)
	}
	if _, err := run(t, "--config", cfg, "config", "set", "graphics=virtio"); err == nil || !strings.Contains(err.Error(), "vmware-svga or reims-vgpu") {
		t.Fatalf("invalid graphics: %v", err)
	}
	if _, err := run(t, "--config", cfg, "config", "set", "graphics=reims-vgpu", "gpu=0000:03:00.0"); err == nil || !strings.Contains(err.Error(), "choose one") {
		t.Fatalf("Reims vGPU with a GPU: %v", err)
	}
	if _, err := run(t, "--config", cfg, "config", "set", "graphics=reims-vgpu"); err != nil {
		t.Fatal(err)
	}
	if out, _ := run(t, "--config", cfg, "config", "show"); !strings.Contains(out, `"graphics": "reims-vgpu"`) || !strings.Contains(out, `"resolution": "1920x1080"`) {
		t.Fatalf("show: %s", out)
	}
	if _, err := run(t, "--config", cfg, "config", "set", "resolution=1600x900"); err == nil || !strings.Contains(err.Error(), "1920x1080, 1920x1200") {
		t.Fatalf("invalid resolution must list the allowed values: %v", err)
	}
	if out, err := run(t, "--config", cfg, "config", "set", "resolution=2560x1440"); err != nil || !strings.Contains(out, "between 2048 and 3840 px") {
		t.Fatalf("a wide resolution before Tahoe warns: %v %s", err, out)
	}
	if out, _ := run(t, "--config", cfg, "config", "show"); !strings.Contains(out, `"resolution": "2560x1440"`) {
		t.Fatalf("show: %s", out)
	}
	if _, err := run(t, "--config", cfg, "media", "remove"); err == nil || !strings.Contains(err.Error(), "--yes") {
		t.Fatalf("media remove needs --yes: %v", err)
	}
	if out, err := run(t, "version"); err != nil || !strings.Contains(out, "OpenCore") {
		t.Fatalf("version: %v %s", err, out)
	}
}

func TestVMCommands(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	cfgFile, vmName = "", ""
	ws := filepath.Join(t.TempDir(), "tahoe")
	if out, err := run(t, "vm", "create", "Tahoe lab", "--version", "tahoe", "--folder", ws); err != nil || !strings.Contains(out, "created and selected Tahoe lab") {
		t.Fatalf("create: %v %s", err, out)
	}
	out, _ := run(t, "vm", "list")
	if !strings.Contains(out, "*  Tahoe lab") {
		t.Fatalf("list must mark the selected VM:\n%s", out)
	}
	if out, err := run(t, "--vm", "Tahoe lab", "config", "show"); err != nil || !strings.Contains(out, `"version": "tahoe"`) {
		t.Fatalf("--vm: %v %s", err, out)
	}
	if _, err := run(t, "config", "set", "nic2.enabled=true", "nic2.mode=bridged", "nic2.bridge=br0", "nic2.model=vmxnet3", "nic2.mac=random"); err != nil {
		t.Fatal(err)
	}
	out, _ = run(t, "config", "show")
	if !strings.Contains(out, `"bridge": "br0"`) || strings.Count(out, `"mac"`) != 2 {
		t.Fatalf("adapter 2 must be saved next to the VM's own adapter:\n%s", out)
	}
	if _, err := run(t, "config", "set", "nic2.mac=01:00:5e:00:00:01"); err == nil {
		t.Fatal("a multicast MAC must be refused")
	}
	root := t.TempDir()
	if _, err := run(t, "workspace", "set", root); err != nil {
		t.Fatal(err)
	}
	if out, err := run(t, "vm", "move", "Tahoe lab"); err != nil || !strings.Contains(out, filepath.Join(root, "tahoe-lab")) {
		t.Fatalf("move: %v %s", err, out)
	}
	if _, err := run(t, "vm", "delete", "Tahoe lab"); err == nil {
		t.Fatal("delete needs --yes")
	}
	if out, err := run(t, "vm", "delete", "Tahoe lab", "--purge", "--yes"); err != nil || !strings.Contains(out, "mac") && !strings.Contains(out, "deleted") {
		t.Fatalf("delete: %v %s", err, out)
	}
	deletePurge, deleteYes = false, false
}

func TestSSHPortCommands(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	cfgFile, vmName = "", ""
	t.Cleanup(func() { createSSHPort = 0; vmName = "" })
	create := func(name string, port int) (string, error) {
		createSSHPort = port
		defer func() { createSSHPort = 0 }()
		return run(t, "vm", "create", name, "--folder", filepath.Join(t.TempDir(), name))
	}
	// No flag: the lowest free port (2222 unless the host holds it).
	want, err := core.NextSSHPort("", nil)
	if err != nil {
		t.Fatal(err)
	}
	if out, err := create("Nilton", 0); err != nil || !strings.Contains(out, fmt.Sprintf("SSH port %d\n", want)) || strings.Contains(out, "belongs to") {
		t.Fatalf("automatic: %v %s", err, out)
	}
	// An unused explicit port is kept.
	if out, err := create("sonoma-test", 2300); err != nil || !strings.Contains(out, "SSH port 2300\n") {
		t.Fatalf("explicit: %v %s", err, out)
	}
	// Another VM's port: the next free one, and the command says why.
	next, _ := core.NextSSHPort("", nil)
	out, err := create("lab", want)
	if err != nil || !strings.Contains(out, fmt.Sprintf("note: SSH port %d belongs to Nilton; using %d", want, next)) {
		t.Fatalf("taken: %v %s", err, out)
	}
	// Out of range: refused, no VM added.
	if _, err := create("bad", 80); err == nil || !strings.Contains(err.Error(), "use a port between 1024 and 65535") {
		t.Fatalf("port 80: %v", err)
	}
	if vms, _ := core.ListVMs(); len(vms) != 3 {
		t.Fatalf("port 80 must add no VM: %d", len(vms))
	}

	// config set ssh-port= to a shared port warns on stderr and still saves (exit 0).
	var stdout, stderr bytes.Buffer
	rootCmd.SetOut(&stdout)
	rootCmd.SetErr(&stderr)
	rootCmd.SetArgs([]string{"--vm", "sonoma-test", "config", "set", fmt.Sprintf("ssh-port=%d", want)})
	if err := rootCmd.Execute(); err != nil || !strings.Contains(stderr.String(), "warning: Also used by Nilton: they cannot run at the same time") ||
		strings.Contains(stdout.String(), "Also used") {
		t.Fatalf("config set: %v\nstdout %s\nstderr %s", err, stdout.String(), stderr.String())
	}
	vmName = ""
	vms, _ := core.ListVMs()
	if e, _ := core.FindVM(vms, "sonoma-test"); e.Config.SSHPort != want {
		t.Fatalf("the shared port must be saved: %d", e.Config.SSHPort)
	}
	if out, err := run(t, "vm", "status", "Nilton"); err != nil || !strings.Contains(out, "warning:   Also used by sonoma-test: they cannot run at the same time") {
		t.Fatalf("status: %v %s", err, out)
	}
	if out, _ := run(t, "vm", "status", "lab"); strings.Contains(out, "warning") {
		t.Fatalf("no warning for an unshared port: %s", out)
	}
}

func TestSettingsAndKextsBeforeStart(t *testing.T) {
	cfg := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", cfg)
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	cfgFile, vmName = "", ""
	if _, err := run(t, "settings", "reset"); err == nil || !strings.Contains(err.Error(), "--yes") {
		t.Fatalf("reset needs --yes: %v", err)
	}
	if out, err := run(t, "settings", "reset", "--yes"); err != nil || !strings.Contains(out, "settings.yaml") {
		t.Fatalf("reset: %v %s", err, out)
	}
	ws := filepath.Join(t.TempDir(), "lab")
	if _, err := run(t, "vm", "create", "Lab", "--version", "sequoia", "--folder", ws); err != nil {
		t.Fatal(err)
	}
	// An entry that cannot be downloaded stops vm start before anything else: no VM runs.
	os.WriteFile(filepath.Join(cfg, "macos-kvm-gui", "settings.yaml"), []byte("kexts:\n  - url: https://127.0.0.1:1/x.zip\n"), 0o644)
	if out, err := run(t, "settings", "show"); err != nil || !strings.Contains(out, "kext: https://127.0.0.1:1/x.zip") || !strings.Contains(out, "kexts downloaded: false") {
		t.Fatalf("show: %v %s", err, out)
	}
	out, err := run(t, "vm", "start")
	if err == nil || !strings.Contains(err.Error(), "127.0.0.1:1") || !strings.Contains(out, "downloading the kexts") {
		t.Fatalf("vm start downloads first: %v %s", err, out)
	}
}

func TestReimsCommands(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("XDG_DATA_HOME", t.TempDir())
	t.Setenv("HOME", t.TempDir()) // the default Workspace, ~/macOS-VMs, may hold a real install
	t.Setenv("MKG_REIMS_QEMU", "")
	t.Cleanup(func() { reimsRelease = "" })
	if _, err := run(t, "reims", "build", "--release", "22.04"); err == nil || !strings.Contains(err.Error(), "24.04 and 26.04") {
		t.Fatalf("an unsupported release: %v", err)
	}
	if _, err := run(t, "reims", "status"); err == nil || !strings.Contains(err.Error(), "not installed") {
		t.Fatalf("nothing installed: %v", err)
	}
	qemu := filepath.Join(t.TempDir(), "20261016-212623-c099dd5a-ubuntu24.04", "bin", "qemu-system-x86_64")
	os.MkdirAll(filepath.Dir(qemu), 0o755)
	os.WriteFile(qemu, []byte("#!/bin/sh\necho 'name \"apple-gfx-pci\", bus PCI'\n"), 0o755)
	t.Setenv("MKG_REIMS_QEMU", qemu)
	out, err := run(t, "reims", "status")
	if err != nil || !strings.Contains(out, "commit: c099dd5a") || !strings.Contains(out, "apple-gfx-pci: yes") {
		t.Fatalf("status: %v\n%s", err, out)
	}
}

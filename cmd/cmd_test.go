package cmd

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
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
	if out, _ := run(t, "--config", cfg, "config", "show"); !strings.Contains(out, `"graphics": "reims-vgpu"`) {
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

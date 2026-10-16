package core

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// fakeProc writes /proc/<pid>/{comm,cmdline} and a cwd link.
func fakeProc(t *testing.T, proc string, pid, comm, cwd string, args ...string) {
	d := filepath.Join(proc, pid)
	write(t, filepath.Join(d, "comm"), comm+"\n")
	write(t, filepath.Join(d, "cmdline"), strings.Join(args, "\x00")+"\x00")
	if cwd != "" {
		os.Symlink(cwd, filepath.Join(d, "cwd"))
	}
}

func TestRunningDetection(t *testing.T) {
	proc := t.TempDir()
	c := Defaults()
	c.Workspace = "/vm/a,b"
	fakeProc(t, proc, "100", "bash", "", "bash", "-c", "file=/vm/a,,b/mac_hdd_ng.img")
	if _, ok := runningIn(proc, c); ok {
		t.Fatal("only qemu processes count")
	}
	fakeProc(t, proc, "200", "qemu-system-x86", "", "qemu-system-x86_64", "-drive", "id=MacHDD,if=none,format=qcow2,file=/vm/a,,b/mac_hdd_ng.img",
		"-vnc", "unix:/run/user/1000/macos-kvm-gui-1/vnc.sock,lock-key-sync=off", "-qmp", "unix:/run/user/1000/macos-kvm-gui-1/qmp.sock,server=on,wait=off",
		"-qmp", "unix:/run/user/1000/macos-kvm-gui-1/qmp-control.sock,server=on,wait=off",
		"-drive", "id=InstallerDisc,if=none,format=raw,snapshot=on,file=/c/InstallAssistant.iso")
	r, ok := runningIn(proc, c)
	if !ok || r.PID != 200 || !r.ByApp || r.VNCSocket != "/run/user/1000/macos-kvm-gui-1/vnc.sock" ||
		r.Control != "/run/user/1000/macos-kvm-gui-1/qmp-control.sock" || !r.Disc || r.Reims {
		t.Fatalf("app-started VM: %+v %v", r, ok)
	}
	// The running process tells whether it is on Reims vGPU, not the config.
	os.RemoveAll(filepath.Join(proc, "200"))
	fakeProc(t, proc, "201", "qemu-system-x86", "", "qemu-system-x86_64", "-drive", "file=/vm/a,,b/mac_hdd_ng.img",
		"-device", "apple-gfx-pci,id=gfx,displays=1,resolution=1920x1080")
	if r, ok := runningIn(proc, c); !ok || !r.Reims {
		t.Fatalf("Reims VM: %+v %v", r, ok)
	}

	// OSX-KVM's OpenCore-Boot.sh: relative, quoted path under its working directory.
	proc2 := t.TempDir()
	o := Defaults()
	o.Workspace = "/home/me/OSX-KVM"
	fakeProc(t, proc2, "300", "qemu-system-x86", "/home/me/OSX-KVM", "qemu-system-x86_64", "-drive", `id=MacHDD,if=none,file="./mac_hdd_ng.img",format=qcow2`)
	if r, ok := runningIn(proc2, o); !ok || r.ByApp {
		t.Fatalf("a VM started outside the app must be found and marked as such: %+v %v", r, ok)
	}
	other := Defaults()
	other.Workspace = "/elsewhere"
	if _, ok := runningIn(proc2, other); ok {
		t.Fatal("another workspace must not match")
	}
}

func TestPreferences(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	if Preference("theme") != "" {
		t.Fatal("unset preference must be empty")
	}
	SetPreference("theme", "light")
	SetPreference("theme", "dark")
	if Preference("theme") != "dark" {
		t.Fatal("the last value must win")
	}
}

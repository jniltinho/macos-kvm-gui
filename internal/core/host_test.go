package core

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
)

func write(t *testing.T, path, data string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(data), 0o644); err != nil {
		t.Fatal(err)
	}
}

// fakeHost builds fake /proc, /sys and /etc in a temp directory.
func fakeHost(t *testing.T, cpuFlags, msrs string, procs ...string) Host {
	root := t.TempDir()
	h := Host{
		ProcRoot: filepath.Join(root, "proc"),
		SysRoot:  filepath.Join(root, "sys"),
		EtcRoot:  filepath.Join(root, "etc"),
		DevKVM:   filepath.Join(root, "kvm"),
		LookPath: func(s string) (string, error) { return "/usr/bin/" + s, nil },
		Output: func(string, ...string) ([]byte, error) {
			return []byte("QEMU emulator version 8.2.2 (Debian 1:8.2.2+ds-0ubuntu1)"), nil
		},
	}
	write(t, filepath.Join(h.ProcRoot, "cpuinfo"), "processor : 0\nflags\t\t: fpu "+cpuFlags+"\n")
	if msrs != "" {
		write(t, h.msrsPath(), msrs+"\n")
	}
	for i, p := range procs {
		write(t, filepath.Join(h.ProcRoot, string(rune('1'+i)), "comm"), p+"\n")
	}
	write(t, h.DevKVM, "")
	return h
}

func byName(cs []Check) map[string]Check {
	m := map[string]Check{}
	for _, c := range cs {
		m[c.Name] = c
	}
	return m
}

func TestChecksReadyHost(t *testing.T) {
	h := fakeHost(t, "svm avx2 sse4_1", "Y", "bash")
	for _, c := range h.Checks("sonoma") {
		if c.Status != OK {
			t.Errorf("%s: %v (%s)", c.Name, c.Status, c.Detail)
		}
	}
}

func TestChecksProblems(t *testing.T) {
	h := fakeHost(t, "fpu", "N", "VBoxHeadless")
	h.LookPath = func(s string) (string, error) {
		if s == "qemu-img" {
			return "", exec.ErrNotFound
		}
		return s, nil
	}
	h.Output = func(string, ...string) ([]byte, error) { return []byte("QEMU emulator version 6.2.0"), nil }
	os.Chmod(h.DevKVM, 0)
	m := byName(h.Checks("sonoma"))
	want := map[string]Status{
		"CPU virtualization":          Fail,
		"qemu-img":                    Fail,
		"QEMU version":                Fail,
		"CPU compatible with version": Fail,
		"kvm ignore_msrs":             Fail,
		"Other hypervisors":           Warn,
	}
	if os.Geteuid() != 0 {
		want["/dev/kvm"] = Fail
	}
	for name, st := range want {
		if m[name].Status != st {
			t.Errorf("%s: expected %v, got %v (%s)", name, st, m[name].Status, m[name].Detail)
		}
	}
	if !strings.Contains(m["qemu-img"].Hint, "qemu-utils") {
		t.Errorf("qemu-img hint must name the package: %q", m["qemu-img"].Hint)
	}
	if !strings.Contains(m["/dev/kvm"].Hint, "kvm") {
		t.Errorf("/dev/kvm hint: %q", m["/dev/kvm"].Hint)
	}
}

func TestMemTotal(t *testing.T) {
	h := fakeHost(t, "svm", "Y")
	write(t, filepath.Join(h.ProcRoot, "meminfo"), "MemFree:  100 kB\nMemTotal:       195037184 kB\n")
	if got := h.MemTotalMiB(); got != 190466 {
		t.Fatalf("MemTotalMiB = %d", got)
	}
}

func TestCompatOldVersionAndUnknownProcs(t *testing.T) {
	h := fakeHost(t, "vmx", "Y")
	if c := h.checkCompat(h.cpuFlags(), "monterey"); c.Status != OK {
		t.Errorf("monterey does not require AVX2: %v", c.Status)
	}
	h.ProcRoot = filepath.Join(h.ProcRoot, "does-not-exist")
	if c := h.checkConflict(); c.Status != Unknown {
		t.Errorf("without /proc it must be inconclusive: %v", c.Status)
	}
}

// exitErr produces a real *exec.ExitError with the given code.
func exitErr(code int) error {
	return exec.Command("/bin/sh", "-c", "exit "+strconv.Itoa(code)).Run()
}

func TestFixIgnoreMSRs(t *testing.T) {
	h := fakeHost(t, "svm", "N")
	conf := filepath.Join(h.EtcRoot, modprobeFile)

	h.Run = func(name string, args ...string) error {
		if name != "pkexec" || args[2] != fixScript {
			t.Fatalf("unexpected command: %s %v", name, args)
		}
		write(t, h.msrsPath(), "Y")
		write(t, conf, "options kvm ignore_msrs=1\n")
		return nil
	}
	if err := h.FixIgnoreMSRs(); err != nil {
		t.Fatalf("expected success: %v", err)
	}

	write(t, h.msrsPath(), "N")
	os.Remove(conf)
	h.Run = func(string, ...string) error { return exitErr(126) }
	if err := h.FixIgnoreMSRs(); !errors.Is(err, ErrCancelled) {
		t.Fatalf("126 must mean cancelled: %v", err)
	}

	h.Run = func(string, ...string) error { write(t, h.msrsPath(), "Y"); return exitErr(1) }
	if err := h.FixIgnoreMSRs(); err == nil || !strings.Contains(err.Error(), "not persist") {
		t.Fatalf("expected partial failure: %v", err)
	}

	write(t, h.msrsPath(), "N")
	h.Run = func(string, ...string) error { return exitErr(127) }
	if err := h.FixIgnoreMSRs(); err == nil || !strings.Contains(err.Error(), "pkexec failed") {
		t.Fatalf("expected pkexec error: %v", err)
	}
}

func TestSetupHost(t *testing.T) {
	h := fakeHost(t, "svm avx2", "N")
	h.LookPath = func(s string) (string, error) {
		if s == "qemu-img" {
			return "", exec.ErrNotFound
		}
		return "/usr/bin/" + s, nil
	}
	h.Run = func(name string, args ...string) error {
		if name != "pkexec" || args[2] != setupScript || args[4] != "nilton" || args[5] != "kvm_amd" || args[6] != "qemu-utils" {
			t.Fatalf("unexpected command: %s %q", name, args[3:])
		}
		write(t, h.msrsPath(), "Y")
		return nil
	}
	msg, err := h.SetupHost("nilton")
	if err != nil || !strings.Contains(msg, "ready") {
		t.Fatalf("setup: %q %v", msg, err)
	}

	os.Chmod(h.DevKVM, 0)
	if os.Geteuid() != 0 {
		if msg, _ := h.SetupHost("nilton"); !strings.Contains(msg, "log out") {
			t.Errorf("new group membership needs a new login: %q", msg)
		}
	}

	h.Run = func(string, ...string) error { return exitErr(126) }
	if _, err := h.SetupHost("nilton"); !errors.Is(err, ErrCancelled) {
		t.Errorf("126 must be cancelled: %v", err)
	}
	h.Run = func(string, ...string) error { t.Fatal("must not elevate"); return nil }
	if _, err := h.SetupHost("bad;user"); err == nil {
		t.Error("invalid user names must be refused before elevation")
	}
	noVirt := fakeHost(t, "fpu", "N")
	noVirt.Run = h.Run
	if _, err := noVirt.SetupHost("nilton"); err == nil || !strings.Contains(err.Error(), "BIOS") {
		t.Errorf("no VT-x/AMD-V: %v", err)
	}
}

// The real shell must reject hostile arguments before running anything.
func TestSetupScriptRejectsInjection(t *testing.T) {
	for _, args := range [][]string{
		{"-o", "kvm_amd", ""},
		{"nilton;reboot", "kvm_amd", ""},
		{"nilton", "kvm_amd;reboot", ""},
		{"nilton", "kvm_amd", "qemu-utils;reboot"},
		{"nilton", "kvm_amd", "$(id)"},
	} {
		err := exec.Command("/bin/sh", append([]string{"-c", setupScript, "sh"}, args...)...).Run()
		var ee *exec.ExitError
		if !errors.As(err, &ee) || ee.ExitCode() != 2 {
			t.Errorf("%q: expected exit 2, got %v", args, err)
		}
	}
}

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

// fakeHost monta /proc, /sys e /etc falsos num diretório temporário.
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
		"Virtualização na CPU":        Fail,
		"qemu-img":                    Fail,
		"Versão do QEMU":              Fail,
		"CPU compatível com a versão": Fail,
		"kvm ignore_msrs":             Fail,
		"Outros hypervisors":          Warn,
	}
	if os.Geteuid() != 0 {
		want["/dev/kvm"] = Fail
	}
	for name, st := range want {
		if m[name].Status != st {
			t.Errorf("%s: esperado %v, veio %v (%s)", name, st, m[name].Status, m[name].Detail)
		}
	}
	if !strings.Contains(m["qemu-img"].Hint, "qemu-utils") {
		t.Errorf("dica do qemu-img deve nomear o pacote: %q", m["qemu-img"].Hint)
	}
	if !strings.Contains(m["/dev/kvm"].Hint, "kvm") {
		t.Errorf("dica do /dev/kvm: %q", m["/dev/kvm"].Hint)
	}
}

func TestCompatOldVersionAndUnknownProcs(t *testing.T) {
	h := fakeHost(t, "vmx", "Y")
	if c := h.checkCompat(h.cpuFlags(), "monterey"); c.Status != OK {
		t.Errorf("monterey não exige AVX2: %v", c.Status)
	}
	h.ProcRoot = filepath.Join(h.ProcRoot, "nao-existe")
	if c := h.checkConflict(); c.Status != Unknown {
		t.Errorf("sem /proc deve ser inconclusivo: %v", c.Status)
	}
}

// exitErr produz um *exec.ExitError real com o código desejado.
func exitErr(code int) error {
	return exec.Command("/bin/sh", "-c", "exit "+strconv.Itoa(code)).Run()
}

func TestFixIgnoreMSRs(t *testing.T) {
	h := fakeHost(t, "svm", "N")
	conf := filepath.Join(h.EtcRoot, modprobeFile)

	h.Run = func(name string, args ...string) error {
		if name != "pkexec" || args[2] != fixScript {
			t.Fatalf("comando inesperado: %s %v", name, args)
		}
		write(t, h.msrsPath(), "Y")
		write(t, conf, "options kvm ignore_msrs=1\n")
		return nil
	}
	if err := h.FixIgnoreMSRs(); err != nil {
		t.Fatalf("sucesso esperado: %v", err)
	}

	write(t, h.msrsPath(), "N")
	os.Remove(conf)
	h.Run = func(string, ...string) error { return exitErr(126) }
	if err := h.FixIgnoreMSRs(); !errors.Is(err, ErrCancelled) {
		t.Fatalf("126 deve ser cancelado: %v", err)
	}

	h.Run = func(string, ...string) error { write(t, h.msrsPath(), "Y"); return exitErr(1) }
	if err := h.FixIgnoreMSRs(); err == nil || !strings.Contains(err.Error(), "não persiste") {
		t.Fatalf("falha parcial esperada: %v", err)
	}

	write(t, h.msrsPath(), "N")
	h.Run = func(string, ...string) error { return exitErr(127) }
	if err := h.FixIgnoreMSRs(); err == nil || !strings.Contains(err.Error(), "pkexec falhou") {
		t.Fatalf("erro do pkexec esperado: %v", err)
	}
}

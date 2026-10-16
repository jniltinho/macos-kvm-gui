package core

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
)

type Status int

const (
	OK Status = iota
	Warn
	Fail
	Unknown
)

func (s Status) String() string {
	return [...]string{"OK", "Aviso", "Falha", "Inconclusivo"}[s]
}

type Check struct {
	Name   string
	Status Status
	Detail string
	Hint   string
}

// Host abstrai o sistema para os testes trocarem /proc, /sys, /etc e os comandos.
type Host struct {
	ProcRoot string
	SysRoot  string
	EtcRoot  string
	DevKVM   string
	LookPath func(string) (string, error)
	Output   func(name string, args ...string) ([]byte, error)
	Run      func(name string, args ...string) error
}

func SystemHost() Host {
	return Host{
		ProcRoot: "/proc",
		SysRoot:  "/sys",
		EtcRoot:  "/etc",
		DevKVM:   "/dev/kvm",
		LookPath: exec.LookPath,
		Output:   func(n string, a ...string) ([]byte, error) { return exec.Command(n, a...).CombinedOutput() },
		Run:      func(n string, a ...string) error { return exec.Command(n, a...).Run() },
	}
}

var tools = []struct{ bin, pkg string }{
	{"qemu-system-x86_64", "qemu-system-x86"},
	{"qemu-img", "qemu-utils"},
	{"git", "git"},
	{"python3", "python3"},
}

// Checks roda todas as verificações do host para a versão de macOS escolhida.
func (h Host) Checks(version string) []Check {
	flags := h.cpuFlags()
	out := []Check{h.checkVirt(flags), h.checkKVM()}
	for _, t := range tools {
		if _, err := h.LookPath(t.bin); err != nil {
			out = append(out, Check{t.bin, Fail, "não encontrado no PATH", "sudo apt install " + t.pkg})
		} else {
			out = append(out, Check{t.bin, OK, "encontrado", ""})
		}
	}
	return append(out, h.checkQEMUVersion(), h.checkCompat(flags, version), h.checkMSRs(), h.checkConflict())
}

// MemTotalMiB lê o MemTotal de /proc/meminfo (0 se não der para ler).
func (h Host) MemTotalMiB() int {
	data, _ := os.ReadFile(filepath.Join(h.ProcRoot, "meminfo"))
	for _, line := range strings.Split(string(data), "\n") {
		if f := strings.Fields(line); len(f) >= 2 && f[0] == "MemTotal:" {
			kb, _ := strconv.Atoi(f[1])
			return kb / 1024
		}
	}
	return 0
}

func (h Host) cpuFlags() map[string]bool {
	data, err := os.ReadFile(filepath.Join(h.ProcRoot, "cpuinfo"))
	if err != nil {
		return nil
	}
	flags := map[string]bool{}
	for _, line := range strings.Split(string(data), "\n") {
		if k, v, ok := strings.Cut(line, ":"); ok && strings.TrimSpace(k) == "flags" {
			for _, f := range strings.Fields(v) {
				flags[f] = true
			}
			break
		}
	}
	return flags
}

func (h Host) checkVirt(flags map[string]bool) Check {
	c := Check{Name: "Virtualização na CPU"}
	switch {
	case flags == nil:
		c.Status, c.Detail = Unknown, "não foi possível ler /proc/cpuinfo"
	case flags["vmx"] || flags["svm"]:
		c.Status, c.Detail = OK, "VT-x/AMD-V presente"
	default:
		c.Status, c.Detail, c.Hint = Fail, "VT-x/AMD-V ausente", "habilite a virtualização na BIOS/UEFI"
	}
	return c
}

func (h Host) checkKVM() Check {
	c := Check{Name: "/dev/kvm"}
	f, err := os.OpenFile(h.DevKVM, os.O_RDWR, 0)
	switch {
	case err == nil:
		f.Close()
		c.Status, c.Detail = OK, "leitura e escrita permitidas"
	case errors.Is(err, os.ErrNotExist):
		c.Status, c.Detail, c.Hint = Fail, "não existe", "carregue o módulo: sudo modprobe kvm_intel (ou kvm_amd)"
	case errors.Is(err, os.ErrPermission):
		c.Status, c.Detail, c.Hint = Fail, "sem permissão", "sudo usermod -aG kvm $USER e faça login de novo"
	default:
		c.Status, c.Detail = Fail, err.Error()
	}
	return c
}

var qemuVersionRe = regexp.MustCompile(`version (\d+)\.(\d+)\.(\d+)`)

func (h Host) checkQEMUVersion() Check {
	c := Check{Name: "Versão do QEMU", Hint: "é preciso QEMU 8.2.2 ou superior"}
	out, err := h.Output("qemu-system-x86_64", "--version")
	m := qemuVersionRe.FindStringSubmatch(string(out))
	if err != nil || m == nil {
		c.Status, c.Detail = Unknown, "não foi possível ler a versão"
		return c
	}
	v := [3]int{}
	for i := range v {
		v[i], _ = strconv.Atoi(m[i+1])
	}
	c.Detail = strings.Join(m[1:], ".")
	if v[0] > 8 || v[0] == 8 && (v[1] > 2 || v[1] == 2 && v[2] >= 2) {
		c.Status, c.Hint = OK, ""
	} else {
		c.Status = Fail
	}
	return c
}

func (h Host) checkCompat(flags map[string]bool, version string) Check {
	c := Check{Name: "CPU compatível com a versão"}
	switch {
	case !ProfileFor(version).NeedsAVX2:
		c.Status, c.Detail = OK, "a versão não exige AVX2"
	case flags == nil:
		c.Status, c.Detail = Unknown, "não foi possível ler as flags da CPU"
	case flags["avx2"] && flags["sse4_1"]:
		c.Status, c.Detail = OK, "SSE4.1 e AVX2 presentes"
	default:
		c.Status, c.Detail, c.Hint = Fail, "Ventura ou posterior exige SSE4.1 e AVX2", "escolha uma versão até Monterey"
	}
	return c
}

func (h Host) msrsPath() string {
	return filepath.Join(h.SysRoot, "module/kvm/parameters/ignore_msrs")
}

func (h Host) checkMSRs() Check {
	c := Check{Name: "kvm ignore_msrs"}
	data, err := os.ReadFile(h.msrsPath())
	switch v := strings.TrimSpace(string(data)); {
	case err != nil:
		c.Status, c.Detail = Unknown, "módulo kvm não carregado"
	case v == "Y" || v == "1":
		c.Status, c.Detail = OK, "ativo"
	default:
		c.Status, c.Detail, c.Hint = Fail, "desativado: o macOS entra em kernel panic", "use o botão Corrigir ignore_msrs"
	}
	return c
}

// hypervisors que, rodando, costumam segurar o VT-x/AMD-V.
var hypervisors = map[string]string{"VBoxHeadless": "VirtualBox", "VirtualBoxVM": "VirtualBox", "vmware-vmx": "VMware"}

func (h Host) checkConflict() Check {
	c := Check{Name: "Outros hypervisors"}
	entries, err := os.ReadDir(h.ProcRoot)
	if err != nil {
		c.Status, c.Detail = Unknown, "não foi possível listar processos"
		return c
	}
	found := map[string]bool{}
	for _, e := range entries {
		if _, err := strconv.Atoi(e.Name()); err != nil {
			continue
		}
		comm, _ := os.ReadFile(filepath.Join(h.ProcRoot, e.Name(), "comm"))
		if name, ok := hypervisors[strings.TrimSpace(string(comm))]; ok {
			found[name] = true
		}
	}
	if len(found) == 0 {
		c.Status, c.Detail = OK, "nenhuma VM de outro hypervisor rodando"
		return c
	}
	names := make([]string, 0, len(found))
	for n := range found {
		names = append(names, n)
	}
	c.Status = Warn
	c.Detail = "VM do " + strings.Join(names, " e ") + " em execução"
	c.Hint = "desligue essa VM antes de iniciar o macOS"
	return c
}

// ErrCancelled indica que o usuário fechou o pedido de senha.
var ErrCancelled = errors.New("correção cancelada")

const modprobeFile = "modprobe.d/macos-kvm-gui.conf"

// fixScript é constante: nada da configuração do usuário entra no shell como root.
const fixScript = `rc=0
echo 1 > /sys/module/kvm/parameters/ignore_msrs || rc=1
printf 'options kvm ignore_msrs=1 report_ignored_msrs=0\n' > /etc/modprobe.d/macos-kvm-gui.conf || rc=1
exit $rc`

// FixIgnoreMSRs aplica a correção via pkexec e confere o estado real depois.
func (h Host) FixIgnoreMSRs() error {
	err := h.Run("pkexec", "/bin/sh", "-c", fixScript)
	var ee *exec.ExitError
	if errors.As(err, &ee) && ee.ExitCode() == 126 {
		return ErrCancelled
	}
	runtimeOK := h.checkMSRs().Status == OK
	conf, _ := os.ReadFile(filepath.Join(h.EtcRoot, modprobeFile))
	persistOK := strings.Contains(string(conf), "ignore_msrs=1")
	switch {
	case runtimeOK && persistOK:
		return nil
	case runtimeOK:
		return fmt.Errorf("ignore_msrs ativo agora, mas falhou ao gravar /etc/%s (não persiste após reiniciar)", modprobeFile)
	case persistOK:
		return fmt.Errorf("/etc/%s gravado, mas ignore_msrs não foi ativado no kernel atual", modprobeFile)
	case err != nil:
		return fmt.Errorf("pkexec falhou: %w", err)
	default:
		return errors.New("a correção não foi aplicada")
	}
}

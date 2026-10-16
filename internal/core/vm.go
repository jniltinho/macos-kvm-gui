package core

import (
	"bufio"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"syscall"
)

// osk é a chave do isa-applesmc, a mesma do OpenCore-Boot.sh.
const osk = "ourhardworkbythesewordsguardedpleasedontsteal(c)AppleComputerInc"

// qemuPath escapa vírgulas, que separam opções do QEMU.
func qemuPath(p string) string { return strings.ReplaceAll(p, ",", ",,") }

// BuildArgs é o OpenCore-Boot.sh portado: config → argumentos do qemu-system-x86_64.
func BuildArgs(c Config, w Workspace, qmpSocket string, withMedia bool) []string {
	p := ProfileFor(c.Version)
	f := func(name string) string { return qemuPath(w.Path(name)) }
	args := []string{
		"-name", "macOS",
		"-enable-kvm", "-m", strconv.Itoa(c.RAMMiB), "-cpu", p.CPU,
		"-machine", "q35",
		"-device", "qemu-xhci,id=xhci",
		"-device", "usb-kbd,bus=xhci.0", "-device", "usb-tablet,bus=xhci.0",
		"-smp", fmt.Sprintf("%d,sockets=1,cores=%d,threads=%d", c.VCPUs, c.Cores, c.VCPUs/c.Cores),
		"-device", "usb-ehci,id=ehci",
		"-device", "isa-applesmc,osk=" + osk,
		"-drive", "if=pflash,format=raw,readonly=on,file=" + f(OVMFCode),
		"-drive", "if=pflash,format=raw,file=" + f(VarsFile),
		"-smbios", "type=2",
		"-device", "ich9-intel-hda", "-device", "hda-duplex",
		"-device", "ich9-ahci,id=sata",
		"-drive", "id=OpenCoreBoot,if=none,snapshot=on,format=qcow2,file=" + f(OpenCore),
		"-device", "ide-hd,bus=sata.2,drive=OpenCoreBoot",
	}
	if withMedia {
		args = append(args,
			"-device", "ide-hd,bus=sata.3,drive=InstallMedia",
			"-drive", "id=InstallMedia,if=none,format=raw,file="+f(MediaFile))
	}
	return append(args,
		"-drive", "id=MacHDD,if=none,format=qcow2,file="+f(DiskFile),
		"-device", "ide-hd,bus=sata.4,drive=MacHDD",
		"-netdev", fmt.Sprintf("user,id=net0,hostfwd=tcp:127.0.0.1:%d-:22", c.SSHPort),
		"-device", p.NIC+",netdev=net0,id=net0,mac=52:54:00:c9:18:27",
		"-device", "vmware-svga",
		"-qmp", "unix:"+qemuPath(qmpSocket)+",server=on,wait=off",
	)
}

// ensureVars cria a NVRAM própria uma única vez; nunca sobrescreve.
func ensureVars(w Workspace) error {
	dst, err := os.OpenFile(w.Path(VarsFile), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o644)
	if errors.Is(err, os.ErrExist) {
		return nil
	}
	if err != nil {
		return err
	}
	src, err := os.Open(w.Path(VarsSource))
	if err != nil {
		dst.Close()
		os.Remove(dst.Name())
		return err
	}
	defer src.Close()
	if _, err := io.Copy(dst, src); err != nil {
		dst.Close()
		os.Remove(dst.Name())
		return err
	}
	return dst.Close()
}

// explainExit traduz a saída do QEMU numa mensagem para o usuário ("" = saída normal).
func explainExit(err error, lastLines []string) string {
	out := strings.Join(lastLines, "\n")
	switch {
	case strings.Contains(out, "failed to initialize kvm: Device or resource busy"):
		return "O KVM está ocupado: outro hypervisor (VirtualBox ou VMware) está usando a virtualização. Desligue a VM dele e tente de novo."
	case strings.Contains(out, `Failed to get "write" lock`):
		return "O disco já está aberto por outro QEMU (iniciado fora do app?). Desligue essa VM antes."
	case err == nil:
		return ""
	case strings.Contains(out, "Could not access KVM kernel module"), strings.Contains(out, "/dev/kvm"):
		return "Sem acesso ao KVM. Confira a aba Host."
	}
	last := ""
	for i := len(lastLines) - 1; i >= 0; i-- { // avisos como o de "pcid" não explicam a saída
		if !strings.Contains(lastLines[i], "warning:") {
			last = ": " + lastLines[i]
			break
		}
	}
	return "O QEMU saiu com erro (" + err.Error() + ")" + last
}

type VMState int

const (
	Stopped VMState = iota
	Running
	Paused
)

func (s VMState) String() string { return [...]string{"Parada", "Em execução", "Pausada por erro"}[s] }

// VM controla um QEMU iniciado por esta instância do app.
type VM struct {
	OnState func(VMState, string) // estado e mensagem (erro ou aviso)
	OnLog   func(string)

	mu      sync.Mutex
	proc    *os.Process
	qmp     *QMP
	state   VMState
	ioError string
	killed  bool
}

func (v *VM) State() VMState {
	v.mu.Lock()
	defer v.mu.Unlock()
	return v.state
}

func (v *VM) setState(s VMState, msg string) {
	v.mu.Lock()
	v.state = s
	v.mu.Unlock()
	if v.OnState != nil {
		v.OnState(s, msg)
	}
}

func (v *VM) log(s string) {
	if v.OnLog != nil {
		v.OnLog(s)
	}
}

// Start valida tudo, pega o lock do workspace (mantido até o QEMU sair) e sobe o QEMU.
func (v *VM) Start(c Config) error {
	if v.State() != Stopped {
		return errors.New("a VM já está em execução")
	}
	if err := c.Validate(); err != nil {
		return err
	}
	dir, _ := c.WorkspaceDir()
	w := NewWorkspace(dir)
	if st, missing := w.Classify(); st != WorkspaceReady {
		return fmt.Errorf("workspace não está pronto (faltam: %s)", strings.Join(missing, ", "))
	}
	if !w.HasDisk() {
		return fmt.Errorf("%s não existe: crie o disco na aba Instalação", DiskFile)
	}
	unlock, err := w.Lock()
	if err != nil {
		return err
	}
	fail := func(err error) error { unlock(); return err }
	if err := ensureVars(w); err != nil {
		return fail(fmt.Errorf("não foi possível criar a NVRAM: %w", err))
	}
	base := os.Getenv("XDG_RUNTIME_DIR")
	if base == "" {
		base = os.TempDir()
	}
	qmpDir, err := os.MkdirTemp(base, "macos-kvm-gui-") // 0700
	if err != nil {
		return fail(err)
	}
	sock := filepath.Join(qmpDir, "qmp.sock")
	args := BuildArgs(c, w, sock, w.HasMedia())
	v.log("qemu-system-x86_64 " + strings.Join(args, " "))

	started := make(chan error, 1)
	go func() {
		// Pdeathsig dispara quando morre a thread que criou o processo: ela fica
		// presa a esta goroutine, que só termina depois do Wait.
		runtime.LockOSThread()
		cmd := exec.Command("qemu-system-x86_64", args...)
		cmd.Dir = dir
		cmd.SysProcAttr = &syscall.SysProcAttr{Pdeathsig: syscall.SIGTERM}
		pr, pw := io.Pipe()
		cmd.Stdout, cmd.Stderr = pw, pw
		if err := cmd.Start(); err != nil {
			started <- err
			return
		}
		v.mu.Lock()
		v.proc, v.ioError = cmd.Process, ""
		v.mu.Unlock()
		v.setState(Running, "") // antes de qualquer Stopped desta mesma goroutine
		started <- nil

		var tail []string
		logDone := make(chan struct{})
		go func() {
			defer close(logDone)
			sc := bufio.NewScanner(pr)
			for sc.Scan() {
				line := sc.Text()
				tail = append(tail, line)
				if len(tail) > 20 {
					tail = tail[1:]
				}
				v.log(line)
			}
		}()
		go v.connectQMP(sock)

		err := cmd.Wait()
		pw.Close()
		<-logDone
		v.mu.Lock()
		q, killed := v.qmp, v.killed
		v.proc, v.qmp, v.killed = nil, nil, false
		v.mu.Unlock()
		if q != nil {
			q.Close()
		}
		os.RemoveAll(qmpDir)
		unlock()
		msg := explainExit(err, tail)
		if killed {
			msg = "Parada forçada pelo usuário."
		}
		v.setState(Stopped, msg)
	}()
	if err := <-started; err != nil {
		os.RemoveAll(qmpDir)
		return fail(fmt.Errorf("não foi possível iniciar o QEMU: %w", err))
	}
	return nil
}

func (v *VM) connectQMP(sock string) {
	q, err := DialQMP(sock, v.onEvent, func() {
		v.mu.Lock()
		quiet := v.killed || v.proc == nil
		v.mu.Unlock()
		if !quiet {
			v.log("aviso: conexão QMP encerrada; o desligamento gracioso fica indisponível")
		}
	})
	if err != nil {
		v.log("aviso: " + err.Error())
		return
	}
	v.mu.Lock()
	if v.proc == nil { // o QEMU já saiu
		v.mu.Unlock()
		q.Close()
		return
	}
	v.qmp = q
	v.mu.Unlock()
}

func (v *VM) onEvent(e QMPEvent) {
	switch e.Name {
	case "BLOCK_IO_ERROR":
		if e.Data["action"] != "stop" {
			return
		}
		msg := fmt.Sprintf("erro de I/O no disco %v (%v)", e.Data["device"], e.Data["operation"])
		if e.Data["nospace"] == true {
			msg += ": sem espaço no host"
		}
		v.mu.Lock()
		v.ioError = msg
		v.mu.Unlock()
		v.setState(Paused, msg)
	case "STOP":
		v.mu.Lock()
		msg := v.ioError
		v.mu.Unlock()
		if msg == "" {
			msg = "o QEMU pausou o convidado"
		}
		v.setState(Paused, msg)
	case "RESUME":
		v.setState(Running, "")
	}
}

// Powerdown pede ao convidado para desligar; o estado só muda quando o QEMU sair.
func (v *VM) Powerdown() error {
	v.mu.Lock()
	q := v.qmp
	v.mu.Unlock()
	if q == nil {
		return errors.New("controle QMP indisponível: use Forçar parada")
	}
	return q.Execute("system_powerdown")
}

// Kill encerra o QEMU na hora (equivale a puxar o cabo).
func (v *VM) Kill() error {
	v.mu.Lock()
	p := v.proc
	if p != nil {
		v.killed = true
	}
	v.mu.Unlock()
	if p == nil {
		return errors.New("a VM não está em execução")
	}
	return p.Kill()
}

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

// osk is the isa-applesmc key, the same as in OpenCore-Boot.sh.
const osk = "ourhardworkbythesewordsguardedpleasedontsteal(c)AppleComputerInc"

// qemuPath escapes commas, which separate QEMU options.
func qemuPath(p string) string { return strings.ReplaceAll(p, ",", ",,") }

// BuildArgs is OpenCore-Boot.sh ported: config → qemu-system-x86_64 arguments.
func BuildArgs(c Config, w Workspace, qmpSocket string, withMedia bool, gpuFuncs []string) []string {
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
		// OpenCore boots from the extracted EFI directory, exposed read-only as a FAT disk by
		// QEMU's vvfat driver. SATA refuses read-only drives, so it sits on virtio-blk.
		"-drive", "id=OpenCoreBoot,if=none,format=raw,readonly=on,file=fat:" + f(EFIDir),
		"-device", "virtio-blk-pci,drive=OpenCoreBoot,bootindex=0",
	}
	if withMedia {
		args = append(args,
			"-device", "ide-hd,bus=sata.3,drive=InstallMedia",
			"-drive", "id=InstallMedia,if=none,format=raw,file="+f(MediaFile))
	}
	args = append(args,
		"-drive", "id=MacHDD,if=none,format=qcow2,file="+f(DiskFile),
		"-device", "ide-hd,bus=sata.4,drive=MacHDD",
		"-netdev", fmt.Sprintf("user,id=net0,hostfwd=tcp:127.0.0.1:%d-:22", c.SSHPort),
		"-device", p.NIC+",netdev=net0,id=net0,mac=52:54:00:c9:18:27",
	)
	// vmware-svga stays even with a passed-through GPU: QEMU's window needs a graphical
	// console to deliver keyboard and mouse input, and it shows the boot screens.
	args = append(args, "-device", "vmware-svga")
	if len(gpuFuncs) > 0 {
		args = append(args, "-device", "vfio-pci,host="+gpuFuncs[0]+",multifunction=on,x-no-kvm-intx=on")
		for _, f := range gpuFuncs[1:] {
			args = append(args, "-device", "vfio-pci,host="+f)
		}
	}
	if c.InApp() {
		// No QEMU window: the screen goes to the app over VNC on a Unix socket in the same
		// private 0700 directory as QMP, never on a TCP port.
		// -k en-us: plain keysyms map to a US (English) layout; with QEMU extended key events
		// the physical scancode goes through and the guest's layout applies. lock-key-sync=off
		// keeps QEMU from injecting Caps Lock presses inferred from the keysym's case.
		args = append(args, "-display", "none", "-vnc", "unix:"+qemuPath(VNCSocket(qmpSocket))+",lock-key-sync=off", "-k", "en-us")
	}
	return append(args, "-qmp", "unix:"+qemuPath(qmpSocket)+",server=on,wait=off")
}

// VNCSocket is the VNC socket next to a VM run's QMP socket.
func VNCSocket(qmpSocket string) string {
	return filepath.Join(filepath.Dir(qmpSocket), "vnc.sock")
}

// memlockLimit is the soft RLIMIT_MEMLOCK in bytes (VFIO pins all guest RAM).
func memlockLimit() uint64 {
	var r syscall.Rlimit
	if err := syscall.Getrlimit(8 /* RLIMIT_MEMLOCK on Linux */, &r); err != nil {
		return 0
	}
	return r.Cur
}

// explainExit turns QEMU output into a message for the user ("" = normal exit).
func explainExit(err error, lastLines []string) string {
	out := strings.Join(lastLines, "\n")
	switch {
	case strings.Contains(out, "failed to initialize kvm: Device or resource busy"):
		return "KVM is busy: another hypervisor (VirtualBox or VMware) is using virtualization. Shut its VM down and try again."
	case strings.Contains(out, `Failed to get "write" lock`):
		return "The disk is already open in another QEMU (started outside the app?). Shut that VM down first."
	case err == nil:
		return ""
	case strings.Contains(out, "Could not access KVM kernel module"), strings.Contains(out, "/dev/kvm"):
		return "No access to KVM. Check the Host tab."
	}
	last := ""
	for i := len(lastLines) - 1; i >= 0; i-- { // warnings such as the "pcid" one do not explain the exit
		if !strings.Contains(lastLines[i], "warning:") {
			last = ": " + lastLines[i]
			break
		}
	}
	return "QEMU exited with an error (" + err.Error() + ")" + last
}

type VMState int

const (
	Stopped VMState = iota
	Running
	Paused
)

func (s VMState) String() string {
	return [...]string{"Stopped", "Running", "Paused (error)"}[s]
}

// VM controls a QEMU started by this app instance.
type VM struct {
	OnState func(VMState, string) // state and message (error or warning)
	OnLog   func(string)

	mu      sync.Mutex
	proc    *os.Process
	qmp     *QMP
	vncSock string // set while running with the in-app display
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

// Start validates everything, takes the workspace lock (held until QEMU exits) and starts QEMU.
func (v *VM) Start(c Config) error {
	if v.State() != Stopped {
		return errors.New("the VM is already running")
	}
	if err := c.Validate(); err != nil {
		return err
	}
	dir, _ := c.WorkspaceDir()
	w := NewWorkspace(dir)
	if !w.Exists() {
		return fmt.Errorf("VM folder %s does not exist: create it in the Install tab", dir)
	}
	if !w.HasDisk() {
		return fmt.Errorf("%s does not exist: create the disk in the Install tab", DiskFile)
	}
	if r, ok := FindRunning(c); ok {
		who := "this app"
		if !r.ByApp {
			who = "another program (for example OSX-KVM's OpenCore-Boot.sh)"
		}
		return fmt.Errorf("this VM's disk is already in use by QEMU (pid %d), started by %s: shut that VM down first", r.PID, who)
	}
	var gpuFuncs []string
	if c.GPU != "" {
		var err error
		if gpuFuncs, err = SystemHost().GPUReady(c, memlockLimit()); err != nil {
			return err
		}
	}
	unlock, err := w.Lock()
	if err != nil {
		return err
	}
	fail := func(err error) error { unlock(); return err }
	if err := w.PrepareBoot(c, w.HasMedia()); err != nil {
		return fail(fmt.Errorf("could not prepare the boot files: %w", err))
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
	args := BuildArgs(c, w, sock, w.HasMedia(), gpuFuncs)
	v.log("qemu-system-x86_64 " + strings.Join(args, " "))

	started := make(chan error, 1)
	go func() {
		// Pdeathsig fires when the thread that created the process dies: it stays
		// locked to this goroutine, which only ends after Wait.
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
		v.proc, v.ioError, v.vncSock = cmd.Process, "", ""
		if c.InApp() {
			v.vncSock = VNCSocket(sock)
		}
		v.mu.Unlock()
		v.setState(Running, "") // before any Stopped from this same goroutine
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
		v.proc, v.qmp, v.killed, v.vncSock = nil, nil, false, ""
		v.mu.Unlock()
		if q != nil {
			q.Close()
		}
		os.RemoveAll(qmpDir)
		unlock()
		msg := explainExit(err, tail)
		if killed {
			msg = "Stopped by the user (forced)."
		}
		v.setState(Stopped, msg)
	}()
	if err := <-started; err != nil {
		os.RemoveAll(qmpDir)
		return fail(fmt.Errorf("could not start QEMU: %w", err))
	}
	return nil
}

func (v *VM) connectQMP(sock string) {
	q, err := DialQMP(sock, v.onEvent, func() {
		v.mu.Lock()
		quiet := v.killed || v.proc == nil
		v.mu.Unlock()
		if !quiet {
			v.log("warning: QMP connection closed; graceful shutdown is unavailable")
		}
	})
	if err != nil {
		v.log("warning: " + err.Error())
		return
	}
	v.mu.Lock()
	if v.proc == nil { // QEMU already exited
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
		msg := fmt.Sprintf("I/O error on disk %v (%v)", e.Data["device"], e.Data["operation"])
		if e.Data["nospace"] == true {
			msg += ": out of space on the host"
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
			msg = "QEMU paused the guest"
		}
		v.setState(Paused, msg)
	case "RESUME":
		v.setState(Running, "")
	}
}

// Powerdown asks the guest to shut down; the state only changes when QEMU exits.
// VNCSocket returns the VNC socket of the running VM, or "" when it is stopped or uses
// QEMU's own window.
func (v *VM) VNCSocket() string {
	v.mu.Lock()
	defer v.mu.Unlock()
	return v.vncSock
}

func (v *VM) Powerdown() error {
	v.mu.Lock()
	q := v.qmp
	v.mu.Unlock()
	if q == nil {
		return errors.New("QMP control unavailable: use Force stop")
	}
	return q.Execute("system_powerdown")
}

// Kill ends QEMU immediately (like pulling the plug).
func (v *VM) Kill() error {
	v.mu.Lock()
	p := v.proc
	if p != nil {
		v.killed = true
	}
	v.mu.Unlock()
	if p == nil {
		return errors.New("the VM is not running")
	}
	return p.Kill()
}

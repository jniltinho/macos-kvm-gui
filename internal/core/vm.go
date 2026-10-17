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
	"time"
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
	if c.Reims() {
		args = reimsArgs(args, c.RAMMiB)
	}
	if withMedia {
		args = append(args,
			"-device", "ide-hd,bus=sata.3,drive=InstallMedia",
			"-drive", "id=InstallMedia,if=none,format=raw,file="+f(MediaFile))
		// The cached full installer as a disc: the recovery installs from it with no network.
		// SATA refuses read-only drives; snapshot=on keeps the guest's writes off the shared file.
		if ci, ok := CachedInstallerFor(c.Version); ok {
			args = append(args,
				"-device", "ide-hd,bus=sata.2,drive=InstallerDisc",
				"-drive", "id=InstallerDisc,if=none,format=raw,snapshot=on,file="+qemuPath(ci.Path))
		}
	}
	args = append(args,
		"-drive", "id=MacHDD,if=none,format=qcow2,file="+f(DiskFile),
		"-device", "ide-hd,bus=sata.4,drive=MacHDD",
	)
	firstNIC, moreNICs := netArgs(c, BridgeHelper())
	args = append(args, firstNIC...)
	// vmware-svga stays even with a passed-through GPU: QEMU's window needs a graphical
	// console to deliver keyboard and mouse input, and it shows the boot screens.
	if c.Reims() {
		// No addr: upstream's 0x4 is taken by ich9-ahci, and OpenCore boots by scanning.
		args = append(args, "-device", "apple-gfx-pci,id=gfx,displays=1,resolution="+c.ReimsResolution())
	} else {
		args = append(args, "-device", "vmware-svga")
	}
	if len(gpuFuncs) > 0 {
		args = append(args, "-device", "vfio-pci,host="+gpuFuncs[0]+",multifunction=on,x-no-kvm-intx=on")
		for _, f := range gpuFuncs[1:] {
			args = append(args, "-device", "vfio-pci,host="+f)
		}
	}
	args = append(args, moreNICs...)
	if c.InApp() || c.Reims() {
		// A Reims VM's screen is always the library's own window (-display none); the VNC
		// socket still serves the Preview and screen capture.
		// No QEMU window: the screen goes to the app over VNC on a Unix socket in the same
		// private 0700 directory as QMP, never on a TCP port.
		// No -k: with it, QEMU remaps extended key events through that layout's keysyms and drops
		// the physical scancode, so keys a US layout lacks (ABNT2's ç and "/?", the ISO key left
		// of Z) were lost. Without it the scancode goes through and macOS's input source applies.
		// lock-key-sync=off keeps QEMU from injecting Caps Lock presses inferred from keysyms.
		args = append(args, "-display", "none", "-vnc", "unix:"+qemuPath(VNCSocket(qmpSocket))+",lock-key-sync=off")
	}
	// A second monitor for one other process (the MCP server or an attached app): QMP serves
	// one client per monitor, and the starter keeps the first.
	return append(args, "-qmp", "unix:"+qemuPath(qmpSocket)+",server=on,wait=off",
		"-qmp", "unix:"+qemuPath(ControlSocket(qmpSocket))+",server=on,wait=off")
}

// ControlSocket is the second QMP socket next to a run's QMP socket.
func ControlSocket(qmpSocket string) string {
	return filepath.Join(filepath.Dir(qmpSocket), "qmp-control.sock")
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
	case strings.Contains(out, "Could not set up host forwarding rule"):
		return "The SSH port is already in use on 127.0.0.1 (another VM?). Choose another port in Settings › Network."
	case strings.Contains(out, "bridge helper") || strings.Contains(out, "access denied by acl file") ||
		strings.Contains(out, "failed to create tun device") || strings.Contains(out, "qemu-bridge-helper"):
		return "A bridged adapter could not attach: qemu-bridge-helper needs to be setuid root (or hold CAP_NET_ADMIN) and the bridge must be allowed in /etc/qemu/bridge.conf. See Troubleshooting › Bridged networking."
	case err == nil:
		return ""
	case udmabufExit(out) != "":
		return udmabufExit(out)
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
	// EmbedX11 (set by the GUI before Start, never by the CLI or MCP): a Reims vGPU VM opens
	// its window on X11 (SDL_VIDEO_DRIVER=x11) so the app can embed it in the Screen tab.
	EmbedX11 bool

	mu       sync.Mutex
	proc     *os.Process
	qmp      *QMP
	vncSock  string // set while running with the in-app display or Reims vGPU
	discDone func() // releases the attached installer disc, if any
	state    VMState
	ioError  string
	killed   bool
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
	if err := checkBridges(c, "/sys/class/net", "/etc/qemu/bridge.conf", BridgeHelper()); err != nil {
		return err
	}
	if err := checkSSHPort(c); err != nil {
		return err
	}
	if c.Extras == nil { // callers that downloaded the kexts pass the snapshot they used
		ks, err := LoadExtraKexts()
		if err != nil {
			return err
		}
		c.Extras = &ks
	}
	var gpuFuncs []string
	if c.GPU != "" {
		var err error
		if gpuFuncs, err = SystemHost().GPUReady(c, memlockLimit()); err != nil {
			return err
		}
	}
	qemu := "qemu-system-x86_64"
	if c.Reims() {
		bin, warnings, err := SystemHost().ReimsReady(c)
		if err != nil {
			return err
		}
		for _, msg := range warnings {
			v.log("warning: Reims vGPU: " + msg)
		}
		qemu = bin
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
	// The installer disc BuildArgs attaches is held (a shared lock) for the whole run, so no
	// process deletes it meanwhile.
	var discDone func()
	if ci, ok := CachedInstallerFor(c.Version); ok && w.HasMedia() {
		if err := RefreshInstallerDisc(ci); err != nil {
			v.log("[installer] " + err.Error())
		}
		if done, err := useDisc(ci.Path); err == nil {
			discDone = done
		}
	}
	args := BuildArgs(c, w, sock, w.HasMedia(), gpuFuncs)
	v.log(qemu + " " + strings.Join(args, " "))

	started := make(chan error, 1)
	go func() {
		// Pdeathsig fires when the thread that created the process dies: it stays
		// locked to this goroutine, which only ends after Wait.
		runtime.LockOSThread()
		cmd := exec.Command(qemu, args...)
		cmd.Dir = dir
		if c.Reims() {
			cmd.Env = reimsEnv(os.Environ(), v.EmbedX11)
		}
		cmd.SysProcAttr = &syscall.SysProcAttr{Pdeathsig: syscall.SIGTERM, Setpgid: true} // Ctrl+C in a terminal must not reach QEMU
		pr, pw := io.Pipe()
		cmd.Stdout, cmd.Stderr = pw, pw
		if err := cmd.Start(); err != nil {
			if discDone != nil {
				discDone()
			}
			started <- err
			return
		}
		v.mu.Lock()
		v.proc, v.ioError, v.vncSock = cmd.Process, "", ""
		if c.InApp() || c.Reims() { // BuildArgs serves VNC for both
			v.vncSock = VNCSocket(sock)
		}
		v.discDone = discDone
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

		startedAt := time.Now()
		err := cmd.Wait()
		w.ClearRootPatchMarker(c, time.Since(startedAt))
		pw.Close()
		<-logDone
		v.mu.Lock()
		q, killed, discDone := v.qmp, v.killed, v.discDone
		v.proc, v.qmp, v.killed, v.vncSock, v.discDone = nil, nil, false, "", nil
		v.mu.Unlock()
		if discDone != nil {
			discDone()
		}
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
// Pid is QEMU's process id while it runs, else 0.
func (v *VM) Pid() int {
	v.mu.Lock()
	defer v.mu.Unlock()
	if v.proc == nil {
		return 0
	}
	return v.proc.Pid
}

func (v *VM) VNCSocket() string {
	v.mu.Lock()
	defer v.mu.Unlock()
	return v.vncSock
}

// InstallerCommand is the line to type in the recovery's Terminal to install from the attached
// installer disc, or "" when the VM has none.
func (v *VM) InstallerCommand() string {
	v.mu.Lock()
	defer v.mu.Unlock()
	if v.discDone == nil {
		return ""
	}
	return DiscCommand
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

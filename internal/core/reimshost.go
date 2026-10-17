package core

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
)

// Reims vGPU's QEMU command and host checks (design: openspec/changes/add-reims-vgpu).

// reimsArgs rewrites the common arguments the way upstream's measured x86-64 command differs:
// guest RAM as a shared memfd (the library maps it), no ACPI S3, no default VGA, xHCI interrupt
// mapping for macOS's driver, and one audio backend bound to the HDA codec and a USB audio
// device (macOS 26 has no HDA driver). apple-gfx-pci replaces vmware-svga in BuildArgs.
func reimsArgs(args []string, ramMiB int) []string {
	out := make([]string, 0, len(args)+16)
	for _, a := range args {
		switch a {
		case "q35":
			out = append(out, "q35,memory-backend=ram",
				"-object", fmt.Sprintf("memory-backend-memfd,id=ram,size=%dM,share=on", ramMiB),
				"-global", "ICH9-LPC.disable_s3=1", "-vga", "none")
		case "qemu-xhci,id=xhci":
			out = append(out, a+",conditional-intr-mapping=on")
		case "hda-duplex":
			// An explicit audiodev leaves no default driver for an unbound codec: bind both.
			out = append(out, "hda-duplex,audiodev=snd0",
				"-audiodev", reimsAudio()+",id=snd0",
				"-device", "usb-audio,audiodev=snd0,bus=xhci.0")
		default:
			out = append(out, a)
		}
	}
	return out
}

// reimsAudio picks the host's sound server: PipeWire, PulseAudio, or none.
var reimsAudio = func() string {
	run := os.Getenv("XDG_RUNTIME_DIR")
	switch {
	case run == "":
		return "none"
	case exists(filepath.Join(run, "pipewire-0")):
		return "pipewire"
	case exists(filepath.Join(run, "pulse", "native")):
		return "pa"
	}
	return "none"
}

// reimsEnv is QEMU's environment without REIMS_VGPU_WINDOW, so the library opens its window;
// with x11 the window is on X11 (SDL_VIDEO_DRIVER=x11) for the app to embed it.
func reimsEnv(env []string, x11 bool) []string {
	out := make([]string, 0, len(env)+1)
	for _, e := range env {
		if !strings.HasPrefix(e, "REIMS_VGPU_WINDOW=") && !(x11 && strings.HasPrefix(e, "SDL_VIDEO_DRIVER=")) {
			out = append(out, e)
		}
	}
	if x11 {
		out = append(out, "SDL_VIDEO_DRIVER=x11")
	}
	return out
}

// reimsMapCount is the vm.max_map_count a VM needs: 64 mappings per MiB of RAM plus a reserve.
func reimsMapCount(ramMiB int) int { return ramMiB*64 + 65536 }

var (
	probeMu    sync.Mutex
	probeCache = map[string]bool{} // path + mtime → lists apple-gfx-pci
)

// hasAppleGfx runs `<qemu> -device help` once per binary version.
func (h Host) hasAppleGfx(qemu string) bool {
	st, err := os.Stat(qemu)
	if err != nil {
		return false
	}
	key := qemu + "\x00" + st.ModTime().String()
	probeMu.Lock()
	defer probeMu.Unlock()
	if ok, done := probeCache[key]; done {
		return ok
	}
	out, _ := h.Output(qemu, "-device", "help")
	ok := strings.Contains(string(out), `"apple-gfx-pci"`)
	probeCache[key] = ok
	return ok
}

// ReimsChecks checks the host for a Reims vGPU VM with ramMiB of RAM.
func (h Host) ReimsChecks(ramMiB int) []Check {
	return []Check{h.checkReimsQEMU(), h.checkVulkan(), h.checkMapCount(ramMiB), h.checkUdmabuf()}
}

func (h Host) checkReimsQEMU() Check {
	c := Check{Name: "Reims vGPU QEMU"}
	qemu, err := ReimsQEMU()
	switch {
	case err != nil:
		c.Status, c.Detail, c.Hint = Fail, err.Error(), "see docs/reims-vgpu.md"
	case !h.hasAppleGfx(qemu):
		c.Status, c.Detail, c.Hint = Fail, qemu+" does not list the apple-gfx-pci device", "install or build Reims vGPU again"
	default:
		c.Status, c.Detail = OK, qemu
	}
	return c
}

func (h Host) checkVulkan() Check {
	c := Check{Name: "Vulkan driver"}
	var found []string
	for _, dir := range []string{filepath.Join(h.ShareRoot, "vulkan/icd.d"), filepath.Join(h.EtcRoot, "vulkan/icd.d")} {
		m, _ := filepath.Glob(filepath.Join(dir, "*.json"))
		for _, f := range m {
			found = append(found, filepath.Base(f))
		}
	}
	if len(found) == 0 {
		c.Status, c.Detail = Fail, "no Vulkan ICD manifest in /usr/share/vulkan/icd.d or /etc/vulkan/icd.d"
		c.Hint = "install your GPU's Vulkan driver (mesa-vulkan-drivers, or the NVIDIA driver)"
		return c
	}
	c.Status, c.Detail = OK, strings.Join(found, ", ")
	return c
}

func (h Host) checkMapCount(ramMiB int) Check {
	need := reimsMapCount(ramMiB)
	c := Check{Name: "vm.max_map_count"}
	data, err := os.ReadFile(filepath.Join(h.ProcRoot, "sys/vm/max_map_count"))
	n, perr := strconv.Atoi(strings.TrimSpace(string(data)))
	switch {
	case err != nil || perr != nil:
		c.Status, c.Detail = Unknown, "could not read /proc/sys/vm/max_map_count"
	case n < need:
		c.Status = Fail
		c.Detail = fmt.Sprintf("%d, below the %d a VM with %d MiB of RAM needs", n, need, ramMiB)
		c.Hint = fmt.Sprintf("press Set up host, or: sudo sysctl -w vm.max_map_count=%d, and to keep it: echo vm.max_map_count=%d | sudo tee /etc/sysctl.d/90-macos-kvm-gui.conf", max(need, 2097152), max(need, 2097152))
	default:
		c.Status, c.Detail = OK, fmt.Sprintf("%d (%d needed for %d MiB)", n, need, ramMiB)
	}
	return c
}

// checkUdmabuf only warns: QEMU may render on another GPU than the AMD one.
func (h Host) checkUdmabuf() Check {
	c := Check{Name: "/dev/udmabuf (AMD GPUs)"}
	amd := false
	vendors, _ := filepath.Glob(filepath.Join(h.SysRoot, "class/drm/card*/device/vendor"))
	for _, v := range vendors {
		if b, _ := os.ReadFile(v); strings.TrimSpace(string(b)) == "0x1002" {
			amd = true
		}
	}
	if !amd {
		c.Status, c.Detail = OK, "no AMD GPU"
		return c
	}
	f, err := os.OpenFile(h.DevUdmabuf, os.O_RDWR, 0)
	switch {
	case err == nil:
		f.Close()
		c.Status, c.Detail = OK, "read and write allowed"
	case errors.Is(err, os.ErrNotExist):
		c.Status, c.Detail, c.Hint = Warn, "does not exist: Vulkan on the AMD GPU may fail", "sudo modprobe udmabuf"
	default:
		c.Status, c.Detail = Warn, "cannot open it read-write: Vulkan on the AMD GPU may fail"
		c.Hint = "join the device's group: " + h.udmabufHint()
	}
	return c
}

// udmabufHint names /dev/udmabuf's group and the command that joins it.
func (h Host) udmabufHint() string {
	group := "the group of /dev/udmabuf"
	if st, err := os.Stat(h.DevUdmabuf); err == nil {
		if sys, ok := st.Sys().(*syscall.Stat_t); ok {
			group = groupName(filepath.Join(h.EtcRoot, "group"), sys.Gid)
		}
	}
	return "sudo usermod -aG " + group + " $USER, then log out and back in"
}

// groupName looks gid up in an /etc/group file, falling back to the number.
func groupName(file string, gid uint32) string {
	data, _ := os.ReadFile(file)
	for _, line := range strings.Split(string(data), "\n") {
		if f := strings.Split(line, ":"); len(f) >= 3 && f[2] == strconv.Itoa(int(gid)) {
			return f[0]
		}
	}
	return strconv.Itoa(int(gid))
}

// ReimsReady runs the checks before a Reims VM starts: the first failure is the error, and
// warnings are returned for the VM log. It returns the QEMU to run.
func (h Host) ReimsReady(c Config) (qemu string, warnings []string, err error) {
	for _, ch := range h.ReimsChecks(c.RAMMiB) {
		msg := ch.Name + ": " + ch.Detail
		if ch.Hint != "" {
			msg += " (" + ch.Hint + ")"
		}
		switch ch.Status {
		case Fail:
			return "", warnings, errors.New("Reims vGPU: " + msg)
		case Warn, Unknown:
			warnings = append(warnings, msg)
		}
	}
	qemu, err = ReimsQEMU()
	return qemu, warnings, err
}

// udmabufExit explains a QEMU exit caused by /dev/udmabuf ("" when it is not one).
func udmabufExit(out string) string {
	if !strings.Contains(out, "udmabuf") {
		return ""
	}
	return "Reims vGPU could not use /dev/udmabuf (needed to render on an AMD GPU): " + SystemHost().udmabufHint() + ", or load it with sudo modprobe udmabuf."
}

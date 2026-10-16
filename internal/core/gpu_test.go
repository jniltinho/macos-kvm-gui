package core

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

type fakeDev struct {
	addr, class, vendor, device, driver, group, bootVGA string
	monitor                                             bool
}

// fakeSysfs builds /sys/bus/pci/devices and /sys/kernel/iommu_groups for the given devices.
func fakeSysfs(t *testing.T, devs []fakeDev) Host {
	h := fakeHost(t, "svm", "Y")
	h.DevVFIO = filepath.Join(h.SysRoot, "dev-vfio")
	for _, d := range devs {
		dir := filepath.Join(h.pciDir(), d.addr)
		write(t, filepath.Join(dir, "class"), "0x"+d.class)
		write(t, filepath.Join(dir, "vendor"), "0x"+d.vendor)
		write(t, filepath.Join(dir, "device"), "0x"+d.device)
		if d.bootVGA != "" {
			write(t, filepath.Join(dir, "boot_vga"), d.bootVGA)
		}
		if d.driver != "" {
			os.Symlink("../../../bus/pci/drivers/"+d.driver, filepath.Join(dir, "driver"))
		}
		if d.monitor {
			card := filepath.Join(h.SysRoot, "class/drm/card"+d.group)
			os.MkdirAll(card, 0o755)
			os.Symlink(dir, filepath.Join(card, "device"))
			write(t, card+"-DP-1/status", "connected")
		}
		if d.driver == "vfio-pci" {
			write(t, filepath.Join(h.DevVFIO, d.group), "")
		}
		gdir := filepath.Join(h.SysRoot, "kernel/iommu_groups", d.group)
		os.MkdirAll(filepath.Join(gdir, "devices"), 0o755)
		os.Symlink(gdir, filepath.Join(dir, "iommu_group"))
		os.Symlink(dir, filepath.Join(gdir, "devices", d.addr))
	}
	return h
}

// referenceHost mirrors the real host, plus a hypothetical RX 6600 in slot 03:00.
var referenceHost = []fakeDev{
	{"0000:01:00.0", "030000", "10de", "2786", "nvidia", "13", "1", true},
	{"0000:01:00.1", "040300", "10de", "22bc", "snd_hda_intel", "13", "", false},
	{"0000:03:00.0", "030000", "1002", "73ff", "amdgpu", "14", "0", false},
	{"0000:03:00.1", "040300", "1002", "ab28", "snd_hda_intel", "14", "", false},
	{"0000:04:04.0", "060400", "1022", "43f5", "pcieport", "17", "", false},
	{"0000:08:00.0", "028000", "8086", "2725", "iwlwifi", "17", "", false},
	{"0000:0a:00.0", "030200", "10de", "1db6", "nvidia", "17", "", false},
	{"0000:75:00.0", "030000", "1002", "13c0", "amdgpu", "29", "0", false},
}

func gpuByAddr(gs []GPU, addr string) GPU {
	for _, g := range gs {
		if g.Addr == addr {
			return g
		}
	}
	return GPU{}
}

func TestGPUListingAndEligibility(t *testing.T) {
	h := fakeSysfs(t, referenceHost)
	gs := h.GPUs()
	if len(gs) != 4 {
		t.Fatalf("expected 4 GPUs, got %d", len(gs))
	}
	cases := []struct {
		addr     string
		compat   Compat
		eligible bool
		reason   string
	}{
		{"0000:01:00.0", CompatUnsupported, false, "no driver"}, // RTX 4070
		{"0000:0a:00.0", CompatUnsupported, false, "no driver"}, // Tesla V100
		{"0000:75:00.0", CompatUnknown, false, "Dortania"},      // Ryzen iGPU must never be reservable
		{"0000:03:00.0", CompatSupported, true, ""},             // RX 6600
	}
	for _, c := range cases {
		g := gpuByAddr(gs, c.addr)
		if g.Compat != c.compat || g.Eligible != c.eligible || !strings.Contains(g.Reason, c.reason) {
			t.Errorf("%s: compat=%v eligible=%v reason=%q", c.addr, g.Compat, g.Eligible, g.Reason)
		}
	}
	rx := gpuByAddr(gs, "0000:03:00.0")
	if len(rx.Funcs) != 2 || rx.Funcs[0].Addr != "0000:03:00.0" || rx.Funcs[1].ID != "1002:ab28" {
		t.Fatalf("slot functions: %+v", rx.Funcs)
	}
}

func TestCompatTable(t *testing.T) {
	cases := map[string]Compat{
		"1002:73ff": CompatSupported,   // RX 6600
		"1002:67df": CompatSupported,   // RX 580
		"1002:73df": CompatUnsupported, // RX 6700 XT: needs NootRX
		"1002:73ef": CompatUnsupported, // RX 6650 XT: needs spoof
		"1002:744c": CompatUnsupported, // RX 7900
		"1002:13c0": CompatUnknown,     // Ryzen 9000 iGPU
		"10de:2786": CompatUnsupported, // RTX 4070
	}
	for id, want := range cases {
		v, d, _ := strings.Cut(id, ":")
		if got := compatOf(v, d); got != want {
			t.Errorf("%s: got %v want %v", id, got, want)
		}
	}
}

func TestEligibilityRefusals(t *testing.T) {
	boot := slices.Clone(referenceHost)
	boot[2].bootVGA = "1"
	if g := gpuByAddr(fakeSysfs(t, boot).GPUs(), "0000:03:00.0"); g.Eligible || !strings.Contains(g.Reason, "primary display") {
		t.Errorf("boot display must be refused: %q", g.Reason)
	}

	shared := slices.Clone(referenceHost)
	shared[2].group, shared[3].group = "17", "17" // same group as the Wi-Fi card
	if g := gpuByAddr(fakeSysfs(t, shared).GPUs(), "0000:03:00.0"); g.Eligible || !strings.Contains(g.Reason, "0000:08:00.0") {
		t.Errorf("shared group must name the foreign device: %q", g.Reason)
	}

	monitor := slices.Clone(referenceHost)
	monitor[2].monitor = true
	if g := gpuByAddr(fakeSysfs(t, monitor).GPUs(), "0000:03:00.0"); g.Eligible || !strings.Contains(g.Reason, "monitor") {
		t.Errorf("GPU driving a monitor must be refused: %q", g.Reason)
	}

	usb := append(slices.Clone(referenceHost), fakeDev{"0000:03:00.2", "0c0330", "1002", "73a6", "xhci_hcd", "14", "", false})
	if g := gpuByAddr(fakeSysfs(t, usb).GPUs(), "0000:03:00.0"); g.Eligible || !strings.Contains(g.Reason, "0000:03:00.2") {
		t.Errorf("a USB function in the slot must be refused: %q", g.Reason)
	}

	audioGroup := slices.Clone(referenceHost)
	audioGroup[3].group = "17" // audio function shares the Wi-Fi card's group
	if g := gpuByAddr(fakeSysfs(t, audioGroup).GPUs(), "0000:03:00.0"); g.Eligible || !strings.Contains(g.Reason, "0000:03:00.1") {
		t.Errorf("every function's group must be checked: %q", g.Reason)
	}

	twin := append(slices.Clone(referenceHost),
		fakeDev{"0000:05:00.0", "030000", "1002", "73ff", "amdgpu", "15", "0", false})
	if g := gpuByAddr(fakeSysfs(t, twin).GPUs(), "0000:03:00.0"); g.Eligible || !strings.Contains(g.Reason, "same PCI ID") {
		t.Errorf("an identical second card must be refused: %q", g.Reason)
	}

	alone := []fakeDev{referenceHost[2], referenceHost[3]}
	if g := gpuByAddr(fakeSysfs(t, alone).GPUs(), "0000:03:00.0"); g.Eligible || !strings.Contains(g.Reason, "host's display") {
		t.Errorf("the host's only GPU must be refused: %q", g.Reason)
	}

	h := fakeSysfs(t, referenceHost)
	os.RemoveAll(filepath.Join(h.SysRoot, "kernel/iommu_groups"))
	if g := gpuByAddr(h.GPUs(), "0000:03:00.0"); g.Eligible || !strings.Contains(g.Reason, "IOMMU") {
		t.Errorf("IOMMU off must be refused: %q", g.Reason)
	}
}

func TestReserveAndRelease(t *testing.T) {
	h := fakeSysfs(t, referenceHost)
	rx := gpuByAddr(h.GPUs(), "0000:03:00.0")
	conf := filepath.Join(h.EtcRoot, vfioModprobe)

	h.Run = func(name string, args ...string) error {
		if name != "pkexec" || args[2] != reserveScript || args[3] != "sh" || args[4] != "1002:73ff,1002:ab28" {
			t.Fatalf("unexpected command: %s %q", name, args)
		}
		write(t, conf, "options vfio-pci ids="+args[4]+" disable_vga=1\n")
		return nil
	}
	if err := h.ReserveGPU(rx); err != nil {
		t.Fatal(err)
	}
	if strings.Contains(reserveScript, "blacklist") {
		t.Fatal("the reservation must never blacklist a driver (the host iGPU uses amdgpu too)")
	}

	h.Run = func(string, ...string) error { return exitErr(126) }
	if err := h.ReleaseGPU(); !errors.Is(err, ErrCancelled) {
		t.Fatalf("126 must be cancelled: %v", err)
	}
	h.Run = func(name string, args ...string) error { os.Remove(conf); return nil }
	if err := h.ReleaseGPU(); err != nil || h.ReservedIDs() != "" {
		t.Fatalf("release: %v", err)
	}

	bad := rx
	bad.Funcs = []PCIFunc{{"0000:03:00.0", "1002:73ff;reboot", "amdgpu"}}
	h.Run = func(string, ...string) error { t.Fatal("must not elevate with an invalid ID"); return nil }
	if err := h.ReserveGPU(bad); err == nil {
		t.Fatal("invalid ID must be refused")
	}
	if err := h.ReserveGPU(gpuByAddr(h.GPUs(), "0000:75:00.0")); err == nil {
		t.Fatal("the iGPU must never be reservable")
	}
}

func TestGPUReady(t *testing.T) {
	c := Defaults()
	c.GPU, c.RAMMiB = "0000:03:00.0", 16384
	devs := slices.Clone(referenceHost)
	if _, err := fakeSysfs(t, devs).GPUReady(c, 1<<40); err == nil || !strings.Contains(err.Error(), "reserve the GPU") {
		t.Fatalf("amdgpu-bound GPU must be refused: %v", err)
	}
	devs[2].driver, devs[3].driver = "vfio-pci", "vfio-pci"
	h := fakeSysfs(t, devs)
	if _, err := h.GPUReady(c, 8<<30); err == nil || !strings.Contains(err.Error(), "16384 MiB") {
		t.Fatalf("low memlock must be refused: %v", err)
	}
	old := c
	old.Version = "big-sur"
	if _, err := h.GPUReady(old, ^uint64(0)); err == nil || !strings.Contains(err.Error(), "monterey") {
		t.Fatalf("RX 6600 on Big Sur must be refused: %v", err)
	}
	funcs, err := h.GPUReady(c, ^uint64(0))
	if err != nil || !slices.Equal(funcs, []string{"0000:03:00.0", "0000:03:00.1"}) {
		t.Fatalf("ready GPU: %v %v", funcs, err)
	}
	os.Chmod(filepath.Join(h.DevVFIO, "14"), 0)
	if os.Geteuid() != 0 {
		if _, err := h.GPUReady(c, ^uint64(0)); err == nil || !strings.Contains(err.Error(), "kvm group") {
			t.Fatalf("unreadable /dev/vfio node must be refused: %v", err)
		}
	}
}

// The real shell must reject hostile IDs before touching anything.
func TestReserveScriptRejectsInjection(t *testing.T) {
	for _, bad := range []string{"", "1002:73ff;reboot", "$(id)", "1002:73ff ab28", "../x"} {
		err := exec.Command("/bin/sh", "-c", reserveScript, "sh", bad).Run()
		var ee *exec.ExitError
		if !errors.As(err, &ee) || ee.ExitCode() != 2 {
			t.Errorf("%q: expected exit 2, got %v", bad, err)
		}
	}
}

func TestBuildArgsWithGPU(t *testing.T) {
	args := BuildArgs(Defaults(), NewWorkspace("/vm"), "/run/q.sock", false, []string{"0000:03:00.0", "0000:03:00.1"})
	if !slices.Contains(args, "vmware-svga") {
		t.Fatal("vmware-svga must stay: the QEMU window needs a graphical console for input")
	}
	if argAfter(args, "-device", "vfio-pci,host=0000:03:00.0,multifunction=on,x-no-kvm-intx=on") == "" ||
		argAfter(args, "-device", "vfio-pci,host=0000:03:00.1") == "" {
		t.Fatalf("vfio args missing: %q", args)
	}
	if !slices.Contains(BuildArgs(Defaults(), NewWorkspace("/vm"), "/run/q.sock", false, nil), "vmware-svga") {
		t.Fatal("without a GPU the emulated display stays")
	}
	c := Defaults()
	c.GPU = "03:00.0"
	var fe *FieldError
	if err := c.Validate(); !errors.As(err, &fe) || fe.Field != "gpu" {
		t.Fatalf("short PCI address must be refused: %v", err)
	}
}

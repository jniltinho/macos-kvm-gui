package core

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
)

type Compat int

const (
	CompatUnknown Compat = iota
	CompatSupported
	CompatUnsupported
	CompatKepler // NVIDIA Kepler: native up to Big Sur, root-patch mode on Monterey to Sequoia
)

func (c Compat) String() string {
	return [...]string{"unknown", "supported by macOS", "not supported by macOS",
		"NVIDIA Kepler: native up to Big Sur; Monterey to Sequoia with root-patch mode (OCLP)"}[c]
}

// amdCompat lists AMD (vendor 1002) device IDs, following the Dortania GPU buyers guide.
// Anything else from AMD, including every APU iGPU, is unknown and therefore not eligible.
var amdCompat = map[string]Compat{
	// Polaris
	"67df": CompatSupported, "67ef": CompatSupported, "67ff": CompatSupported,
	// Vega, Radeon VII
	"687f": CompatSupported, "66af": CompatSupported,
	// Navi 10/14
	"731f": CompatSupported, "7340": CompatSupported,
	// Navi 21 (RX 6800/6800 XT/6900 XT), Navi 23 (RX 6600/6600 XT)
	"73bf": CompatSupported, "73ff": CompatSupported,
	// Navi 24, RDNA3: no macOS driver
	"743f": CompatUnsupported,
	"744c": CompatUnsupported, "7448": CompatUnsupported, "747e": CompatUnsupported, "7480": CompatUnsupported,
}

// amdNeedsPatch only work with a device-id spoof or the NootRX kext, which this app does not set up:
// Lexa, RX 580 2048SP, Navi 21 XTXH/RX 6950 XT, Navi 22 (RX 6700/6750), Navi 23 RX 6650 XT.
var amdNeedsPatch = map[string]bool{"699f": true, "6fdf": true, "73af": true, "73a5": true, "73df": true, "73ef": true}

func compatOf(vendor, device string) Compat {
	switch vendor {
	case "1002":
		if amdNeedsPatch[device] {
			return CompatUnsupported
		}
		return amdCompat[device] // zero value is CompatUnknown
	case "10de":
		if IsKepler(vendor + ":" + device) {
			return CompatKepler
		}
		return CompatUnsupported // Maxwell and newer: no Metal driver since High Sierra's Web Drivers
	case "8086":
		return CompatUnsupported
	}
	return CompatUnknown
}

// PCIFunc is one function of the GPU's slot (the GPU itself, its HDMI audio, ...).
type PCIFunc struct {
	Addr   string // 0000:03:00.1
	ID     string // 1002:ab28
	Driver string
}

type GPU struct {
	Addr     string // address of the display function, 0000:03:00.0
	ID       string // vendor:device
	Name     string
	Driver   string
	Group    string
	BootVGA  bool
	Monitor  bool // a DRM connector of this GPU has a monitor attached
	Compat   Compat
	Funcs    []PCIFunc // every function of the slot, display function first
	Eligible bool
	Reason   string // why it is not eligible
}

var (
	pciAddrRe = regexp.MustCompile(`^[0-9a-f]{4}:[0-9a-f]{2}:[0-9a-f]{2}\.[0-7]$`)
	pciIDRe   = regexp.MustCompile(`^[0-9a-f]{4}:[0-9a-f]{4}$`)
)

func (h Host) pciDir() string { return filepath.Join(h.SysRoot, "bus/pci/devices") }

func (h Host) pciAttr(addr, name string) string {
	b, _ := os.ReadFile(filepath.Join(h.pciDir(), addr, name))
	return strings.TrimPrefix(strings.TrimSpace(string(b)), "0x")
}

func (h Host) pciLink(addr, name string) string {
	t, err := os.Readlink(filepath.Join(h.pciDir(), addr, name))
	if err != nil {
		return ""
	}
	return filepath.Base(t)
}

func (h Host) pciFunc(addr string) PCIFunc {
	return PCIFunc{addr, h.pciAttr(addr, "vendor") + ":" + h.pciAttr(addr, "device"), h.pciLink(addr, "driver")}
}

func slotOf(addr string) string { return addr[:strings.LastIndexByte(addr, '.')] }

// GPUs lists the host's display adapters (PCI class 0x03xxxx) with their eligibility.
func (h Host) GPUs() []GPU {
	entries, _ := os.ReadDir(h.pciDir())
	var all []string
	for _, e := range entries {
		all = append(all, e.Name())
	}
	slices.Sort(all)
	groups, _ := os.ReadDir(filepath.Join(h.SysRoot, "kernel/iommu_groups"))
	iommu := len(groups) > 0
	connected := h.connectedGPUs()

	var gpus []GPU
	for _, addr := range all {
		if !strings.HasPrefix(h.pciAttr(addr, "class"), "03") {
			continue
		}
		f := h.pciFunc(addr)
		g := GPU{Addr: addr, ID: f.ID, Driver: f.Driver, Group: h.pciLink(addr, "iommu_group"),
			BootVGA: h.pciAttr(addr, "boot_vga") == "1", Monitor: connected[addr]}
		vendor, device, _ := strings.Cut(f.ID, ":")
		g.Compat = compatOf(vendor, device)
		g.Name = h.pciName(addr, f.ID)
		g.Funcs = []PCIFunc{f}
		for _, other := range all {
			if other != addr && slotOf(other) == slotOf(addr) {
				g.Funcs = append(g.Funcs, h.pciFunc(other))
			}
		}
		gpus = append(gpus, g)
	}
	for i := range gpus {
		gpus[i].Eligible, gpus[i].Reason = h.eligibility(gpus[i], gpus, all, iommu)
	}
	return gpus
}

// connectedGPUs maps PCI addresses to whether any of their DRM connectors has a monitor attached.
func (h Host) connectedGPUs() map[string]bool {
	out := map[string]bool{}
	conns, _ := filepath.Glob(filepath.Join(h.SysRoot, "class/drm/card*-*/status"))
	for _, st := range conns {
		b, _ := os.ReadFile(st)
		if strings.TrimSpace(string(b)) != "connected" {
			continue
		}
		card := strings.SplitN(filepath.Base(filepath.Dir(st)), "-", 2)[0]
		if dev, err := filepath.EvalSymlinks(filepath.Join(h.SysRoot, "class/drm", card, "device")); err == nil {
			out[filepath.Base(dev)] = true
		}
	}
	return out
}

func (h Host) groupMembers(group string) []string {
	var members []string
	ms, _ := os.ReadDir(filepath.Join(h.SysRoot, "kernel/iommu_groups", group, "devices"))
	for _, m := range ms {
		members = append(members, m.Name())
	}
	return members
}

func (h Host) pciName(addr, id string) string {
	if h.Output != nil {
		if out, err := h.Output("lspci", "-s", addr); err == nil {
			if _, name, ok := strings.Cut(strings.TrimSpace(string(out)), ": "); ok {
				return name
			}
		}
	}
	return "PCI device " + id
}

// needsPatchKext reports whether macOS drives the GPU only with a patch kext.
func needsPatchKext(id string) bool {
	vendor, device, _ := strings.Cut(id, ":")
	return vendor == "1002" && amdNeedsPatch[device]
}

// patchKextReason is why a patch-dependent GPU cannot be used with this settings.yaml ("" = it can).
func patchKextReason(ks ExtraKexts, err error) string {
	switch {
	case err != nil:
		return err.Error()
	case len(ks.Entries) == 0:
		return "macOS drives this GPU only with a patch kext: list one in " + SettingsFile() + " (see docs/gpu-passthrough.md)"
	}
	return ""
}

func (h Host) eligibility(g GPU, gpus []GPU, all []string, iommu bool) (bool, string) {
	patch, patchReason := needsPatchKext(g.ID), ""
	switch {
	case patch && h.extras != nil:
		patchReason = patchKextReason(*h.extras, nil)
	case patch:
		patchReason = patchKextReason(LoadExtraKexts())
	}
	switch {
	case !iommu:
		return false, "IOMMU is disabled: enable AMD-Vi (or VT-d) in the BIOS"
	case patchReason != "":
		return false, patchReason
	case g.Compat == CompatUnsupported && !patch:
		return false, "macOS has no driver for this GPU"
	case g.Compat == CompatUnknown:
		return false, "not in the list of macOS-supported GPUs; AMD Ryzen iGPUs have no macOS driver (see the Dortania GPU buyers guide)"
	case g.BootVGA:
		return false, "this GPU shows the host's boot screen: make the integrated GPU the primary display in the BIOS"
	case g.Monitor:
		return false, "a monitor is connected to this GPU and the host is using it: move the host's monitor to another GPU first"
	}
	hostDisplay := false
	for _, o := range gpus {
		if slotOf(o.Addr) != slotOf(g.Addr) && o.Driver != "" && o.Driver != "vfio-pci" {
			hostDisplay = true
		}
	}
	if !hostDisplay {
		return false, "no other GPU with a driver is left for the host's display"
	}
	ids := map[string]bool{}
	for _, f := range g.Funcs {
		if !pciIDRe.MatchString(f.ID) {
			return false, "unexpected PCI ID " + f.ID
		}
		if class := h.pciAttr(f.Addr, "class"); !strings.HasPrefix(class, "03") && !strings.HasPrefix(class, "0403") {
			return false, f.Addr + " (class " + class + ", for example a USB-C controller) is not a GPU or audio function; passing it through is not supported"
		}
		ids[f.ID] = true
		for _, m := range h.groupMembers(h.pciLink(f.Addr, "iommu_group")) {
			if slotOf(m) != slotOf(g.Addr) && !strings.HasPrefix(h.pciAttr(m, "class"), "0604") {
				return false, "the IOMMU group of " + f.Addr + " also contains " + m + ", from another slot: move the card to another slot"
			}
		}
	}
	for _, other := range all {
		if slotOf(other) != slotOf(g.Addr) && ids[h.pciFunc(other).ID] {
			return false, other + " has the same PCI ID, and vfio-pci would claim it too"
		}
	}
	return true, ""
}

// Files written by the reservation.
const (
	vfioModprobe = "modprobe.d/macos-kvm-gui-vfio.conf"
	vfioUdev     = "udev/rules.d/99-macos-kvm-gui-vfio.rules"
)

// reserveScript is constant: the IDs arrive as $1 and are checked again here. It holds a
// host-wide lock and rolls the files back if the initramfs cannot be rebuilt.
const reserveScript = `case "$1" in ""|*[!0-9a-f:,]*) echo "invalid PCI IDs" >&2; exit 2;; esac
exec 9>/run/lock/macos-kvm-gui-host.lock && flock -n 9 || { echo "another GPU change is running" >&2; exit 3; }
printf 'options vfio-pci ids=%s disable_vga=1\nsoftdep amdgpu pre: vfio-pci\nsoftdep radeon pre: vfio-pci\nsoftdep nouveau pre: vfio-pci\nsoftdep nvidia pre: vfio-pci\nsoftdep snd_hda_intel pre: vfio-pci\n' "$1" > /etc/modprobe.d/macos-kvm-gui-vfio.conf || exit 1
printf 'SUBSYSTEM=="vfio", GROUP="kvm", MODE="0660"\n' > /etc/udev/rules.d/99-macos-kvm-gui-vfio.rules || exit 1
if ! update-initramfs -u -k all; then
  rm -f /etc/modprobe.d/macos-kvm-gui-vfio.conf /etc/udev/rules.d/99-macos-kvm-gui-vfio.rules
  update-initramfs -u -k all
  echo "initramfs rebuild failed; the reservation was rolled back" >&2
  exit 1
fi`

const releaseScript = `exec 9>/run/lock/macos-kvm-gui-host.lock && flock -n 9 || { echo "another GPU change is running" >&2; exit 3; }
rm -f /etc/modprobe.d/macos-kvm-gui-vfio.conf /etc/udev/rules.d/99-macos-kvm-gui-vfio.rules || exit 1
update-initramfs -u -k all || { echo "files removed, but the initramfs rebuild failed: run sudo update-initramfs -u -k all" >&2; exit 1; }`

func vfioIDs(g GPU) (string, error) {
	var ids []string
	for _, f := range g.Funcs {
		if !pciIDRe.MatchString(f.ID) {
			return "", fmt.Errorf("refusing unexpected PCI ID %q", f.ID)
		}
		if !slices.Contains(ids, f.ID) {
			ids = append(ids, f.ID)
		}
	}
	return strings.Join(ids, ","), nil
}

// ReservedIDs returns the IDs currently reserved for vfio-pci by this app ("" = none).
func (h Host) ReservedIDs() string {
	b, _ := os.ReadFile(filepath.Join(h.EtcRoot, vfioModprobe))
	m := regexp.MustCompile(`ids=([0-9a-f:,]+)`).FindSubmatch(b)
	if m == nil {
		return ""
	}
	return string(m[1])
}

func (h Host) runPrivileged(script string, args ...string) error {
	err := h.Run("pkexec", append([]string{"/bin/sh", "-c", script, "sh"}, args...)...)
	if pkexecCancelled(err) {
		return ErrCancelled
	}
	return err
}

// ReserveGPU makes vfio-pci claim the GPU's slot at the next boot.
func (h Host) ReserveGPU(g GPU) error {
	if !g.Eligible {
		return errors.New("this GPU cannot be passed through: " + g.Reason)
	}
	ids, err := vfioIDs(g)
	if err != nil {
		return err
	}
	if err := h.runPrivileged(reserveScript, ids); err != nil {
		if errors.Is(err, ErrCancelled) {
			return err
		}
		return fmt.Errorf("reservation failed: %w", err)
	}
	if h.ReservedIDs() != ids {
		return errors.New("reservation did not take effect: /etc/" + vfioModprobe + " was not written")
	}
	return nil
}

// ReleaseGPU removes the reservation; the GPU gets its normal driver back after a reboot.
func (h Host) ReleaseGPU() error {
	if err := h.runPrivileged(releaseScript); err != nil {
		if errors.Is(err, ErrCancelled) {
			return err
		}
		return fmt.Errorf("release failed: %w", err)
	}
	if h.ReservedIDs() != "" {
		return errors.New("release did not take effect: /etc/" + vfioModprobe + " still exists")
	}
	return nil
}

// gpuMinVersion is the oldest macOS with a native driver for the GPU (Dortania).
var gpuMinVersion = map[string]string{
	"731f": "catalina", "7340": "catalina", // Navi 10/14: 10.15.1+
	"73bf": "big-sur",  // Navi 21: 11.4+
	"73ff": "monterey", // Navi 23: 12.1+
	"66af": "mojave",   // Radeon VII: 10.14.5+
}

// GPUReady checks, right before QEMU starts, that the chosen GPU can really be used.
// It returns the slot's functions, display function first.
func (h Host) GPUReady(c Config, memlock uint64) ([]string, error) {
	ks := ExtraKexts{}
	if c.Extras != nil {
		ks = *c.Extras
	}
	h.extras = &ks
	var g *GPU
	for _, cand := range h.GPUs() {
		if cand.Addr == c.GPU {
			g = &cand
			break
		}
	}
	switch {
	case g == nil:
		return nil, fmt.Errorf("GPU %s not found on this host", c.GPU)
	case !g.Eligible:
		return nil, errors.New("GPU " + c.GPU + " cannot be passed through: " + g.Reason)
	}
	if min, ok := gpuMinVersion[strings.TrimPrefix(g.ID, "1002:")]; ok && versionIndex(c.Version) < versionIndex(min) {
		return nil, fmt.Errorf("this GPU needs macOS %s or newer", min)
	}
	if err := keplerReady(*g, c); err != nil {
		return nil, err
	}
	var funcs []string
	for _, f := range g.Funcs {
		if f.Driver != "vfio-pci" {
			return nil, fmt.Errorf("%s is bound to %q, not vfio-pci: reserve the GPU in the GPU tab and reboot the host", f.Addr, f.Driver)
		}
		node := filepath.Join(h.DevVFIO, h.pciLink(f.Addr, "iommu_group"))
		fd, err := os.OpenFile(node, os.O_RDWR, 0)
		if err != nil {
			return nil, fmt.Errorf("cannot open %s (%v): check the udev rule from the reservation and that you are in the kvm group (log in again after joining it)", node, err)
		}
		fd.Close()
		funcs = append(funcs, f.Addr)
	}
	if need := uint64(c.RAMMiB) << 20; memlock < need {
		return nil, fmt.Errorf("the soft locked-memory limit is %d MiB but the VM needs %d MiB (VFIO pins all guest RAM): raise memlock in /etc/security/limits.d and log in again", memlock>>20, c.RAMMiB)
	}
	return funcs, nil
}

// keplerReady checks a Kepler GPU against the VM's macOS: native up to Big Sur, root-patch mode
// from Monterey to Sequoia, nothing on Tahoe.
func keplerReady(g GPU, c Config) error {
	switch {
	case g.Compat != CompatKepler, versionIndex(c.Version) <= versionIndex("big-sur"):
		return nil
	case !RootPatchVersion(c.Version):
		return errors.New("NVIDIA Kepler GPUs have no driver on " + versionDisplay(c.Version) + ", and OCLP does not patch it yet: use Big Sur to Sequoia")
	case !c.RootPatch:
		return errors.New("on " + versionDisplay(c.Version) + " a Kepler GPU needs root-patch mode (GPU tab) and OCLP's root patch inside macOS")
	}
	return nil
}

// Package core holds the app logic, with no dependency on the UI.
package core

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// Version is a macOS version accepted by fetch-macOS-v2.py (-s <Short>).
type Version struct {
	Short string
	Name  string
}

// Versions in release order; the index decides the hardware profile.
var Versions = []Version{
	{"high-sierra", "High Sierra (10.13)"},
	{"mojave", "Mojave (10.14)"},
	{"catalina", "Catalina (10.15)"},
	{"big-sur", "Big Sur (11)"},
	{"monterey", "Monterey (12)"},
	{"ventura", "Ventura (13)"},
	{"sonoma", "Sonoma (14)"},
	{"sequoia", "Sequoia (15)"},
	{"tahoe", "Tahoe (26)"},
}

const (
	idxBigSur   = 3
	idxMonterey = 4
	idxVentura  = 5
	// validated: the only version tested end to end on the reference host.
	validated = "sonoma"
)

func versionIndex(short string) int {
	for i, v := range Versions {
		if v.Short == short {
			return i
		}
	}
	return -1
}

// Config is what the user adjusts and the app persists.
type Config struct {
	Name      string `json:"name,omitempty"` // shown in the VM list
	Workspace string `json:"workspace"`
	Version   string `json:"version"`
	RAMMiB    int    `json:"ram_mib"`
	Cores     int    `json:"cores"`
	VCPUs     int    `json:"vcpus"`
	SSHPort   int    `json:"ssh_port"`
	GPU       string `json:"gpu,omitempty"`     // PCI address of a passed-through GPU; "" = emulated display
	Display   string `json:"display,omitempty"` // "app" (default): screen inside the app over VNC; "window": QEMU's own window
	// Networks are the adapters (Adapter 1–4). nil (JSON null or absent), for VMs saved before
	// adapters existed, means one NAT adapter with the legacy MAC: see Adapters. An empty list
	// means no adapter at all, so it must survive JSON: no omitempty.
	Networks []NIC `json:"networks"`
	// RootPatch (experimental, off by default) lowers SIP and loads OCLP's AMFIPass and helpers
	// so OpenCore Legacy Patcher can root-patch a passed-through NVIDIA Kepler GPU inside
	// Monterey to Sequoia (see rootpatch.go).
	RootPatch bool `json:"root_patch,omitempty"`
	// Graphics is the display adapter: "" or GraphicsSVGA (the default), or GraphicsReims, a
	// paravirtual GPU with Metal through the host's Vulkan driver (reims.go, reimshost.go).
	Graphics string `json:"graphics,omitempty"`
	// rootPatchWasOn is set by PrepareBoot when the previous boot had root-patch mode on.
	rootPatchWasOn bool
	// Extras is a start's snapshot of settings.yaml, never saved; nil
	// (creating a VM) loads no extra kext. extraKexts is set by PrepareBoot: {folder, name}.
	Extras     *ExtraKexts `json:"-"`
	extraKexts [][2]string
	// VerboseBoot is a one-off start option, never saved: macOS prints its boot log on screen
	// (-v) and stops on a panic with its details (debug=0x100) instead of showing the logo.
	VerboseBoot bool `json:"-"`
}

func Defaults() Config {
	return Config{Workspace: "~/OSX-KVM", Version: validated, RAMMiB: 8192, Cores: 4, VCPUs: 8, SSHPort: 2222}
}

// Recommended fits CPU and RAM to the host. Without a supported GPU macOS draws the
// screen on the CPU, so more vCPUs make the display visibly smoother.
func Recommended(c Config, hostThreads, hostMemMiB int) Config {
	// About half the host's threads, as cores × 2 threads, with a power-of-two core count
	// (see Validate): 24 host threads give 4 cores × 2 = 8 vCPUs.
	want := min(max(hostThreads/2, 2), 16)
	cores := 1
	for cores*2*2 <= want {
		cores *= 2
	}
	c.VCPUs, c.Cores = cores*2, cores
	c.RAMMiB = min(max(hostMemMiB/4/1024*1024, MinRAMMiB), 16384)
	return c
}

// Display modes.
const (
	DisplayApp    = "app"
	DisplayWindow = "window"
)

// Graphics adapters.
const (
	GraphicsSVGA  = "vmware-svga"
	GraphicsReims = "reims-vgpu"
)

// Reims reports whether the VM runs on Reims vGPU.
func (c Config) Reims() bool { return c.Graphics == GraphicsReims }

// InApp reports whether the VM's screen is shown inside the app (the default).
func (c Config) InApp() bool { return c.Display != DisplayWindow }

// FieldError names the invalid field for the UI to highlight.
type FieldError struct {
	Field string
	Msg   string
}

func (e *FieldError) Error() string { return e.Field + ": " + e.Msg }

// MinRAMMiB is the least guest RAM macOS installs with.
const MinRAMMiB = 4096

// WorkspaceDir expands "~" and requires an absolute path.
func (c Config) WorkspaceDir() (string, error) {
	p := c.Workspace
	if p == "~" || strings.HasPrefix(p, "~/") {
		home, err := os.UserHomeDir()
		if err != nil {
			return "", err
		}
		p = filepath.Join(home, strings.TrimPrefix(p, "~"))
	}
	if !filepath.IsAbs(p) {
		return "", &FieldError{"workspace", "must be an absolute path"}
	}
	return filepath.Clean(p), nil
}

func (c Config) Validate() error {
	if _, err := c.WorkspaceDir(); err != nil {
		return err
	}
	if versionIndex(c.Version) < 0 {
		return &FieldError{"version", "unknown version: " + c.Version}
	}
	if c.RAMMiB < MinRAMMiB {
		return &FieldError{"ram", fmt.Sprintf("minimum is %d MiB", MinRAMMiB)}
	}
	if c.Cores < 1 {
		return &FieldError{"cores", "minimum is 1 core"}
	}
	if c.VCPUs < 1 || c.VCPUs%c.Cores != 0 {
		return &FieldError{"vcpus", "must be a multiple of the number of cores"}
	}
	// macOS stops at the Apple logo, right after the boot loader hands over (EXITBS:START),
	// when the core count is not a power of two: 6 cores × 2 threads never boots.
	if c.Cores&(c.Cores-1) != 0 {
		return &FieldError{"cores", fmt.Sprintf("%d cores: use 1, 2, 4, 8 or 16 (macOS hangs at the Apple logo with other core counts)", c.Cores)}
	}
	if c.Display != "" && c.Display != DisplayApp && c.Display != DisplayWindow {
		return &FieldError{"display", "expected app or window"}
	}
	if c.RootPatch && !RootPatchVersion(c.Version) {
		return &FieldError{"root_patch", "root-patch mode is for Monterey 12 to Sequoia 15: Big Sur drives Kepler GPUs natively, and Tahoe has no stable OCLP support yet"}
	}
	if c.Graphics != "" && c.Graphics != GraphicsSVGA && c.Graphics != GraphicsReims {
		return &FieldError{"graphics", "expected " + GraphicsSVGA + " or " + GraphicsReims}
	}
	if c.Reims() && c.GPU != "" {
		return &FieldError{"graphics", "Reims vGPU and GPU passthrough cannot be used together: choose one of them (clear the GPU, or set graphics to " + GraphicsSVGA + ")"}
	}
	if c.GPU != "" && !pciAddrRe.MatchString(c.GPU) {
		return &FieldError{"gpu", "expected a PCI address such as 0000:03:00.0"}
	}
	if c.SSHPort < 1024 || c.SSHPort > 65535 {
		return &FieldError{"ssh_port", "use a port between 1024 and 65535"}
	}
	return validateNICs(c.Adapters())
}

// ConfigPath is ~/.config/macos-kvm-gui/config.json (or the XDG equivalent).
func ConfigPath() (string, error) {
	dir, err := os.UserConfigDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "macos-kvm-gui", "config.json"), nil
}

// LoadConfig returns the defaults when the file does not exist yet.
func LoadConfig(path string) (Config, error) {
	c := Defaults()
	data, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return c, nil
	}
	if err != nil {
		return c, err
	}
	if err := json.Unmarshal(data, &c); err != nil {
		return Defaults(), fmt.Errorf("invalid config in %s: %w", path, err)
	}
	return c, nil
}

// SaveConfig validates and writes atomically (temp file + rename).
func SaveConfig(path string, c Config) error {
	if err := c.Validate(); err != nil {
		return err
	}
	data, err := json.MarshalIndent(c, "", "  ")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	return writeFileAtomic(path, data, 0o644)
}

func writeFileAtomic(path string, data []byte, perm os.FileMode) error {
	tmp, err := os.CreateTemp(filepath.Dir(path), ".tmp-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	if err := os.Chmod(tmp.Name(), perm); err != nil {
		return err
	}
	return os.Rename(tmp.Name(), path)
}

// Profile is the virtual hardware derived from the macOS version.
type Profile struct {
	CPU          string
	Experimental bool
	NeedsAVX2    bool
}

// cpuFlags comes from MY_OPTIONS in OpenCore-Boot.sh.
const cpuFlags = "kvm=on,vendor=GenuineIntel,+invtsc,vmware-cpuid-freq=on,+ssse3,+sse4.2,+popcnt,+avx,+aes,+xsave,+xsaveopt,check"

func ProfileFor(version string) Profile {
	i := versionIndex(version)
	p := Profile{
		CPU:          "Skylake-Client,-hle,-rtm," + cpuFlags,
		Experimental: version != validated,
		NeedsAVX2:    i >= idxVentura,
	}
	if i <= idxMonterey {
		p.CPU = "Penryn," + cpuFlags
	}
	return p
}

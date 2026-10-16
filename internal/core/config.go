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
}

func Defaults() Config {
	return Config{Workspace: "~/OSX-KVM", Version: validated, RAMMiB: 8192, Cores: 4, VCPUs: 8, SSHPort: 2222}
}

// Recommended fits CPU and RAM to the host. Without a supported GPU macOS draws the
// screen on the CPU, so more vCPUs make the display visibly smoother.
func Recommended(c Config, hostThreads, hostMemMiB int) Config {
	vcpus := min(max(hostThreads/2/2*2, 2), 16)
	c.VCPUs, c.Cores = vcpus, max(vcpus/2, 1)
	c.RAMMiB = min(max(hostMemMiB/4/1024*1024, 4096), 16384)
	return c
}

// Display modes.
const (
	DisplayApp    = "app"
	DisplayWindow = "window"
)

// InApp reports whether the VM's screen is shown inside the app (the default).
func (c Config) InApp() bool { return c.Display != DisplayWindow }

// FieldError names the invalid field for the UI to highlight.
type FieldError struct {
	Field string
	Msg   string
}

func (e *FieldError) Error() string { return e.Field + ": " + e.Msg }

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
	if c.RAMMiB < 4096 {
		return &FieldError{"ram", "minimum is 4096 MiB"}
	}
	if c.Cores < 1 {
		return &FieldError{"cores", "minimum is 1 core"}
	}
	if c.VCPUs < 1 || c.VCPUs%c.Cores != 0 {
		return &FieldError{"vcpus", "must be a multiple of the number of cores"}
	}
	if c.Display != "" && c.Display != DisplayApp && c.Display != DisplayWindow {
		return &FieldError{"display", "expected app or window"}
	}
	if c.GPU != "" && !pciAddrRe.MatchString(c.GPU) {
		return &FieldError{"gpu", "expected a PCI address such as 0000:03:00.0"}
	}
	if c.SSHPort < 1024 || c.SSHPort > 65535 {
		return &FieldError{"ssh_port", "use a port between 1024 and 65535"}
	}
	return nil
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
	NIC          string
	Experimental bool
	NeedsAVX2    bool
}

// cpuFlags comes from MY_OPTIONS in OpenCore-Boot.sh.
const cpuFlags = "kvm=on,vendor=GenuineIntel,+invtsc,vmware-cpuid-freq=on,+ssse3,+sse4.2,+popcnt,+avx,+aes,+xsave,+xsaveopt,check"

func ProfileFor(version string) Profile {
	i := versionIndex(version)
	p := Profile{
		CPU:          "Skylake-Client,-hle,-rtm," + cpuFlags,
		NIC:          "virtio-net-pci",
		Experimental: version != validated,
		NeedsAVX2:    i >= idxVentura,
	}
	if i <= idxMonterey {
		p.CPU = "Penryn," + cpuFlags
	}
	if version == "high-sierra" {
		p.NIC = "vmxnet3"
	}
	return p
}

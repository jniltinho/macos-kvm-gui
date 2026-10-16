package core

import (
	"bufio"
	"crypto/rand"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

// Network adapters, like VirtualBox's Adapter 1–4: each is NAT (QEMU user mode) or bridged to a
// host bridge through qemu-bridge-helper, with a model and a MAC address.

const (
	MaxNICs    = 4
	NetNAT     = "nat"
	NetBridged = "bridged"
	// legacyMAC is the address every VM used before adapters were configurable (it comes from
	// OSX-KVM). VMs without an adapter list keep it, so macOS sees the same en0.
	legacyMAC = "52:54:00:c9:18:27"
)

// NICModels are the adapter types the app offers; see NICSupported for which macOS has a driver.
var NICModels = []string{"virtio-net-pci", "vmxnet3", "e1000-82545em"}

// NIC is one network adapter.
type NIC struct {
	Enabled bool   `json:"enabled"`
	Mode    string `json:"mode"`             // NetNAT or NetBridged
	Bridge  string `json:"bridge,omitempty"` // host bridge for NetBridged, e.g. br0
	Model   string `json:"model"`
	MAC     string `json:"mac"`
}

// NICSupported reports whether stock macOS of that version has a driver for the model:
// VirtIO networking arrived in Big Sur, the Intel 82545EM driver was removed in Ventura and
// vmxnet3 works throughout.
func NICSupported(version, model string) bool {
	i := versionIndex(version)
	switch model {
	case "virtio-net-pci":
		return i >= idxBigSur
	case "e1000-82545em":
		return i < idxVentura
	}
	return model == "vmxnet3"
}

// defaultNICModel is the adapter type a new adapter gets for the macOS version.
func defaultNICModel(version string) string {
	if versionIndex(version) >= idxBigSur {
		return "virtio-net-pci"
	}
	return "vmxnet3"
}

// Adapters is the VM's effective adapter list: its own, or for a VM saved before adapters
// existed, one NAT adapter with the version's model and the legacy MAC.
func (c Config) Adapters() []NIC {
	if c.Networks != nil {
		return c.Networks
	}
	return []NIC{{Enabled: true, Mode: NetNAT, Model: defaultNICModel(c.Version), MAC: legacyMAC}}
}

// NIC returns adapter i (0-based) for editing. A legacy VM gets its implicit adapter written
// out, and missing adapters up to i are added disabled, with the version's model and a fresh MAC.
func (c *Config) NIC(i int) (*NIC, error) {
	if i < 0 || i >= MaxNICs {
		return nil, fmt.Errorf("adapters are numbered 1 to %d", MaxNICs)
	}
	if c.Networks == nil {
		c.Networks = c.Adapters()
	}
	for len(c.Networks) <= i {
		c.Networks = append(c.Networks, NIC{Mode: NetNAT, Model: defaultNICModel(c.Version), MAC: RandomMAC()})
	}
	return &c.Networks[i], nil
}

// NewNetworks is the adapter list of a new VM: one NAT adapter with a fresh MAC.
func NewNetworks(version string) []NIC {
	return []NIC{{Enabled: true, Mode: NetNAT, Model: defaultNICModel(version), MAC: RandomMAC()}}
}

// RandomMAC is a random locally administered unicast address.
func RandomMAC() string {
	b := make([]byte, 6)
	rand.Read(b)
	b[0] = b[0]&0xfc | 0x02
	return net.HardwareAddr(b).String()
}

var bridgeNameRe = regexp.MustCompile(`^[A-Za-z0-9_.-]{1,15}$`)

// validateNICs checks the adapter list: known modes and models, a valid bridge name and
// unique, locally administered unicast MAC addresses.
func validateNICs(nics []NIC) error {
	if len(nics) > MaxNICs {
		return &FieldError{"networks", fmt.Sprintf("at most %d adapters", MaxNICs)}
	}
	seen := map[string]int{}
	for i, n := range nics {
		field := fmt.Sprintf("nic%d", i+1)
		if n.Mode != NetNAT && n.Mode != NetBridged {
			return &FieldError{field, "attached to must be nat or bridged"}
		}
		if n.Mode == NetBridged && !bridgeNameRe.MatchString(n.Bridge) {
			return &FieldError{field, "choose the host bridge, for example br0"}
		}
		ok := false
		for _, m := range NICModels {
			ok = ok || n.Model == m
		}
		if !ok {
			return &FieldError{field, "adapter type must be one of " + strings.Join(NICModels, ", ")}
		}
		hw, err := net.ParseMAC(n.MAC)
		if err != nil || len(hw) != 6 || !strings.Contains(n.MAC, ":") {
			return &FieldError{field, n.MAC + " is not a MAC address such as 52:54:00:12:34:56"}
		}
		if hw[0]&0x03 != 0x02 {
			return &FieldError{field, n.MAC + " must be a locally administered unicast address (second hex digit 2, 6, A or E)"}
		}
		key := hw.String()
		if j, dup := seen[key]; dup {
			return &FieldError{field, fmt.Sprintf("%s is already used by adapter %d", n.MAC, j+1)}
		}
		seen[key] = i
	}
	return nil
}

// netArgs are the QEMU arguments for the adapters. Adapter 1 keeps the position it always had
// (QEMU numbers it among the other devices); adapters 2–4 sit on fixed PCI slots so adding one
// never moves the display or a passed-through GPU. With no enabled adapter, QEMU must not add
// its default one. The SSH forward goes on the first enabled NAT adapter; further NAT adapters
// get their own subnet.
func netArgs(c Config, helper string) (first, rest []string) {
	any, forwarded := false, false
	for i, n := range c.Adapters() {
		if !n.Enabled {
			continue
		}
		any = true
		id := fmt.Sprintf("net%d", i)
		var netdev string
		switch n.Mode {
		case NetBridged:
			netdev = fmt.Sprintf("bridge,id=%s,br=%s,helper=%s", id, n.Bridge, qemuPath(helper))
		default:
			netdev = "user,id=" + id
			if i > 0 {
				netdev += fmt.Sprintf(",net=10.0.%d.0/24", 2+i)
			}
			if !forwarded {
				netdev += fmt.Sprintf(",hostfwd=tcp:127.0.0.1:%d-:22", c.SSHPort)
				forwarded = true
			}
		}
		dev := fmt.Sprintf("%s,netdev=%s,id=%s,mac=%s", n.Model, id, id, strings.ToLower(n.MAC))
		if i == 0 {
			first = append(first, "-netdev", netdev, "-device", dev)
		} else {
			rest = append(rest, "-netdev", netdev, "-device", fmt.Sprintf("%s,bus=pcie.0,addr=0x%x", dev, 0x15+i))
		}
	}
	if !any {
		first = []string{"-nic", "none"}
	}
	return first, rest
}

// SSHForwarded reports whether the VM has the localhost SSH forward (an enabled NAT adapter).
func (c Config) SSHForwarded() bool {
	for _, n := range c.Adapters() {
		if n.Enabled && n.Mode == NetNAT {
			return true
		}
	}
	return false
}

// bridgeHelpers are where distributions install qemu-bridge-helper (Debian/Ubuntu, Fedora).
var bridgeHelpers = []string{"/usr/lib/qemu/qemu-bridge-helper", "/usr/libexec/qemu-bridge-helper"}

// BridgeHelper is the installed qemu-bridge-helper, or "".
func BridgeHelper() string {
	for _, p := range bridgeHelpers {
		if st, err := os.Stat(p); err == nil && st.Mode().IsRegular() {
			return p
		}
	}
	return ""
}

// HostBridges lists the host's Linux bridges (interfaces with a bridge/ directory in sysfs).
func HostBridges() []string { return bridgesIn("/sys/class/net") }

func bridgesIn(root string) []string {
	var out []string
	entries, _ := os.ReadDir(root)
	for _, e := range entries {
		if st, err := os.Stat(filepath.Join(root, e.Name(), "bridge")); err == nil && st.IsDir() {
			out = append(out, e.Name())
		}
	}
	return out
}

// checkBridges is the preflight for bridged adapters. It refuses what certainly cannot work and
// leaves the rest to QEMU, whose error explainExit translates: the ACL file may be unreadable to
// the user, and the helper may hold a file capability instead of the setuid bit.
func checkBridges(c Config, netRoot, aclFile, helper string) error {
	for i, n := range c.Adapters() {
		if !n.Enabled || n.Mode != NetBridged {
			continue
		}
		name := fmt.Sprintf("Adapter %d", i+1)
		if st, err := os.Stat(filepath.Join(netRoot, n.Bridge, "bridge")); err != nil || !st.IsDir() {
			return fmt.Errorf("%s: the host has no bridge named %s (see Troubleshooting › Bridged networking)", name, n.Bridge)
		}
		if helper == "" {
			return fmt.Errorf("%s: qemu-bridge-helper is not installed (it comes with QEMU's common package)", name)
		}
		if allowed, known := bridgeAllowed(aclFile, n.Bridge); known && !allowed {
			return fmt.Errorf("%s: %s does not allow %s. As root, add the line \"allow %s\" to %s", name, aclFile, n.Bridge, n.Bridge, aclFile)
		}
	}
	return nil
}

// bridgeAllowed applies qemu-bridge-helper's ACL rules to bridge: "allow NAME|all",
// "deny NAME|all" and "include FILE"; a matching deny wins over any allow. known is false when
// the file cannot be read, so the decision is left to the helper.
func bridgeAllowed(aclFile, bridge string) (allowed, known bool) {
	var allow, deny bool
	seen := map[string]bool{}
	var read func(path string) bool
	read = func(path string) bool {
		if seen[path] || len(seen) > 16 {
			return true
		}
		seen[path] = true
		f, err := os.Open(path)
		if err != nil {
			return false
		}
		defer f.Close()
		sc := bufio.NewScanner(f)
		for sc.Scan() {
			fields := strings.Fields(sc.Text())
			if len(fields) < 2 || strings.HasPrefix(fields[0], "#") {
				continue
			}
			match := fields[1] == "all" || fields[1] == bridge
			switch fields[0] {
			case "allow":
				allow = allow || match
			case "deny":
				deny = deny || match
			case "include":
				if !read(fields[1]) {
					return false
				}
			}
		}
		return true
	}
	if !read(aclFile) {
		return false, false
	}
	return allow && !deny, true
}

// ErrPortInUse is returned when the SSH port is already taken, often by another VM.
var ErrPortInUse = errors.New("port in use")

// checkSSHPort fails when the SSH forward's port is taken on 127.0.0.1, before QEMU would.
func checkSSHPort(c Config) error {
	if !c.SSHForwarded() {
		return nil
	}
	l, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", c.SSHPort))
	if err != nil {
		return fmt.Errorf("SSH port %d on 127.0.0.1 is in use (another VM?): choose another port in Settings › Network: %w", c.SSHPort, ErrPortInUse)
	}
	l.Close()
	return nil
}

// DiskVirtualSize reads the virtual size of a qcow2 disk from its header (bytes 24–31), or 0.
func DiskVirtualSize(path string) int64 {
	f, err := os.Open(path)
	if err != nil {
		return 0
	}
	defer f.Close()
	h := make([]byte, 32)
	if n, _ := f.Read(h); n < 32 || string(h[:4]) != "QFI\xfb" {
		return 0
	}
	var size int64
	for _, b := range h[24:32] {
		size = size<<8 | int64(b)
	}
	return size
}

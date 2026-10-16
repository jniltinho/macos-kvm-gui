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
	return [...]string{"OK", "Warning", "Failure", "Inconclusive"}[s]
}

type Check struct {
	Name   string
	Status Status
	Detail string
	Hint   string
}

// Host abstracts the system so tests can swap /proc, /sys, /etc and the commands.
type Host struct {
	ProcRoot string
	SysRoot  string
	EtcRoot  string
	DevKVM   string
	DevVFIO  string
	LookPath func(string) (string, error)
	Output   func(name string, args ...string) ([]byte, error)
	Run      func(name string, args ...string) error

	// ShareRoot is /usr/share (Vulkan ICD manifests); DevUdmabuf is /dev/udmabuf.
	ShareRoot, DevUdmabuf string
	// extras, when set, is the start's kexts snapshot that eligibility uses instead of
	// reading settings.yaml again.
	extras *ExtraKexts
}

func SystemHost() Host {
	return Host{
		ProcRoot: "/proc",
		SysRoot:  "/sys",
		EtcRoot:  "/etc",
		DevKVM:   "/dev/kvm",
		DevVFIO:  "/dev/vfio",
		LookPath: exec.LookPath,
		Output:   func(n string, a ...string) ([]byte, error) { return exec.Command(n, a...).CombinedOutput() },
		Run: func(n string, a ...string) error {
			out, err := exec.Command(n, a...).CombinedOutput()
			if msg := strings.Join(strings.Fields(string(out)), " "); err != nil && msg != "" {
				return fmt.Errorf("%w: %s", err, msg) // keeps *exec.ExitError reachable via errors.As
			}
			return err
		},
		ShareRoot:  "/usr/share",
		DevUdmabuf: "/dev/udmabuf",
	}
}

var tools = []struct{ bin, pkg string }{
	{"qemu-system-x86_64", "qemu-system-x86"},
	{"qemu-img", "qemu-utils"},
}

// Checks runs every host check for the chosen macOS version.
func (h Host) Checks(version string) []Check { return h.ChecksFor(Config{Version: version}) }

// ChecksFor runs every host check for c: the base checks, then ReimsChecksFor's.
func (h Host) ChecksFor(c Config) []Check {
	return append(h.baseChecks(c.Version), h.ReimsChecksFor(c)...)
}

// ReimsChecksFor runs Reims vGPU's checks for c, as warnings at most unless c runs on Reims vGPU.
func (h Host) ReimsChecksFor(c Config) []Check {
	ram := c.RAMMiB
	if ram == 0 {
		ram = Defaults().RAMMiB
	}
	out := h.ReimsChecks(ram)
	for i := range out {
		if out[i].Status == Fail && !c.Reims() {
			out[i].Status = Warn
		}
	}
	return out
}

func (h Host) baseChecks(version string) []Check {
	flags := h.cpuFlags()
	out := []Check{h.checkVirt(flags), h.checkKVM()}
	for _, t := range tools {
		if _, err := h.LookPath(t.bin); err != nil {
			out = append(out, Check{t.bin, Fail, "not found in PATH", "sudo apt install " + t.pkg})
		} else {
			out = append(out, Check{t.bin, OK, "found", ""})
		}
	}
	return append(out, h.checkQEMUVersion(), h.checkCompat(flags, version), h.checkMSRs(), h.checkConflict())
}

// MemTotalMiB reads MemTotal from /proc/meminfo (0 if it cannot be read).
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
	c := Check{Name: "CPU virtualization"}
	switch {
	case flags == nil:
		c.Status, c.Detail = Unknown, "could not read /proc/cpuinfo"
	case flags["vmx"] || flags["svm"]:
		c.Status, c.Detail = OK, "VT-x/AMD-V present"
	default:
		c.Status, c.Detail, c.Hint = Fail, "VT-x/AMD-V missing", "enable virtualization in the BIOS/UEFI"
	}
	return c
}

func (h Host) checkKVM() Check {
	c := Check{Name: "/dev/kvm"}
	f, err := os.OpenFile(h.DevKVM, os.O_RDWR, 0)
	switch {
	case err == nil:
		f.Close()
		c.Status, c.Detail = OK, "read and write allowed"
	case errors.Is(err, os.ErrNotExist):
		c.Status, c.Detail, c.Hint = Fail, "does not exist", "use Set up host (loads kvm_amd or kvm_intel)"
	case errors.Is(err, os.ErrPermission):
		c.Status, c.Detail, c.Hint = Fail, "permission denied", "use Set up host (adds you to the kvm group), then log out and back in"
	default:
		c.Status, c.Detail = Fail, err.Error()
	}
	return c
}

var qemuVersionRe = regexp.MustCompile(`version (\d+)\.(\d+)\.(\d+)`)

func (h Host) checkQEMUVersion() Check {
	c := Check{Name: "QEMU version", Hint: "QEMU 8.2.2 or later is required"}
	out, err := h.Output("qemu-system-x86_64", "--version")
	m := qemuVersionRe.FindStringSubmatch(string(out))
	if err != nil || m == nil {
		c.Status, c.Detail = Unknown, "could not read the version"
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
	c := Check{Name: "CPU compatible with version"}
	switch {
	case !ProfileFor(version).NeedsAVX2:
		c.Status, c.Detail = OK, "this version does not require AVX2"
	case flags == nil:
		c.Status, c.Detail = Unknown, "could not read the CPU flags"
	case flags["avx2"] && flags["sse4_1"]:
		c.Status, c.Detail = OK, "SSE4.1 and AVX2 present"
	default:
		c.Status, c.Detail, c.Hint = Fail, "Ventura or later requires SSE4.1 and AVX2", "choose a version up to Monterey"
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
		c.Status, c.Detail = Unknown, "kvm module not loaded"
	case v == "Y" || v == "1":
		c.Status, c.Detail = OK, "enabled"
	default:
		c.Status, c.Detail, c.Hint = Fail, "disabled: macOS will kernel panic", "use Set up host"
	}
	return c
}

// hypervisors that, while running, tend to hold VT-x/AMD-V.
var hypervisors = map[string]string{"VBoxHeadless": "VirtualBox", "VirtualBoxVM": "VirtualBox", "vmware-vmx": "VMware"}

func (h Host) checkConflict() Check {
	c := Check{Name: "Other hypervisors"}
	entries, err := os.ReadDir(h.ProcRoot)
	if err != nil {
		c.Status, c.Detail = Unknown, "could not list processes"
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
		c.Status, c.Detail = OK, "no VM from another hypervisor running"
		return c
	}
	names := make([]string, 0, len(found))
	for n := range found {
		names = append(names, n)
	}
	c.Status = Warn
	c.Detail = strings.Join(names, " and ") + " VM running"
	c.Hint = "shut that VM down before starting macOS"
	return c
}

// ErrCancelled means the user closed or refused the password prompt.
var ErrCancelled = errors.New("cancelled at the password prompt")

// pkexecCancelled tells a dismissed or refused password prompt apart from real errors:
// pkexec exits 126 when the dialog is dismissed, and the KDE agent makes it exit 127 with
// "Not authorized"; any other 127 (for example a missing program) stays an error.
func pkexecCancelled(err error) bool {
	var ee *exec.ExitError
	if !errors.As(err, &ee) {
		return false
	}
	return ee.ExitCode() == 126 || ee.ExitCode() == 127 && strings.Contains(err.Error(), "Not authorized")
}

const modprobeFile = "modprobe.d/macos-kvm-gui.conf"

// fixScript is constant: nothing from the user's config reaches the root shell.
const fixScript = `rc=0
echo 1 > /sys/module/kvm/parameters/ignore_msrs || rc=1
printf 'options kvm ignore_msrs=1 report_ignored_msrs=0\n' > /etc/modprobe.d/macos-kvm-gui.conf || rc=1
exit $rc`

// FixIgnoreMSRs applies the fix via pkexec and then checks the real state.
func (h Host) FixIgnoreMSRs() error {
	err := h.Run("pkexec", "/bin/sh", "-c", fixScript)
	if pkexecCancelled(err) {
		return ErrCancelled
	}
	runtimeOK := h.checkMSRs().Status == OK
	conf, _ := os.ReadFile(filepath.Join(h.EtcRoot, modprobeFile))
	persistOK := strings.Contains(string(conf), "ignore_msrs=1")
	switch {
	case runtimeOK && persistOK:
		return nil
	case runtimeOK:
		return fmt.Errorf("ignore_msrs is enabled now, but writing /etc/%s failed (it will not persist after reboot)", modprobeFile)
	case persistOK:
		return fmt.Errorf("/etc/%s written, but ignore_msrs was not enabled in the running kernel", modprobeFile)
	case err != nil:
		return fmt.Errorf("pkexec failed: %w", err)
	default:
		return errors.New("the fix was not applied")
	}
}

// setupScript is constant: user, KVM module and packages arrive as $1, $2 and $3 and are
// checked again here. Packages are installed only when some are missing.
const setupScript = `case "$1" in ""|-*|*[!a-z0-9_.-]*) echo "invalid user name" >&2; exit 2;; esac
case "$2" in kvm_intel|kvm_amd) ;; *) echo "invalid KVM module" >&2; exit 2;; esac
case "$3" in *[!a-z0-9.+\ -]*) echo "invalid package list" >&2; exit 2;; esac
if [ -n "$3" ]; then
  command -v apt-get >/dev/null || { echo "apt-get not found: install $3 with your package manager" >&2; exit 3; }
  DEBIAN_FRONTEND=noninteractive apt-get install -y $3 || exit 1
fi
[ -e /dev/kvm ] || modprobe "$2" || { echo "could not load $2: enable virtualization in the BIOS" >&2; exit 1; }
getent group kvm >/dev/null || groupadd --system kvm || exit 1
usermod -aG kvm "$1" || exit 1
echo 1 > /sys/module/kvm/parameters/ignore_msrs || exit 1
printf 'options kvm ignore_msrs=1 report_ignored_msrs=0\n' > /etc/modprobe.d/macos-kvm-gui.conf || exit 1
# Reims vGPU maps guest memory in many small views: 2097152 covers a VM of up to 30 GiB.
if [ "$(cat /proc/sys/vm/max_map_count)" -lt 2097152 ]; then
  printf 'vm.max_map_count = 2097152\n' > /etc/sysctl.d/90-macos-kvm-gui.conf || exit 1
  sysctl -qw vm.max_map_count=2097152 || exit 1
fi`

var userNameRe = regexp.MustCompile(`^[a-z_][a-z0-9_.-]*$`)

// SetupHost does, with one password prompt, the basics KVM needs: missing QEMU packages,
// the KVM module, kvm group membership (which also opens /dev/udmabuf on Ubuntu), ignore_msrs
// and a vm.max_map_count large enough for Reims vGPU. It returns what the user still has
// to do, such as logging in again for the new group to apply.
func (h Host) SetupHost(user string) (string, error) {
	if !userNameRe.MatchString(user) {
		return "", fmt.Errorf("unexpected user name %q", user)
	}
	flags := h.cpuFlags()
	module := ""
	switch {
	case flags["svm"]:
		module = "kvm_amd"
	case flags["vmx"]:
		module = "kvm_intel"
	default:
		return "", errors.New("the CPU reports no VT-x/AMD-V: enable virtualization in the BIOS/UEFI first")
	}
	var pkgs []string
	for _, t := range tools {
		if _, err := h.LookPath(t.bin); err != nil {
			pkgs = append(pkgs, t.pkg)
		}
	}
	err := h.Run("pkexec", "/bin/sh", "-c", setupScript, "sh", user, module, strings.Join(pkgs, " "))
	if pkexecCancelled(err) {
		return "", ErrCancelled
	}
	if err != nil {
		return "", fmt.Errorf("host setup failed: %w", err)
	}
	if h.checkMSRs().Status != OK {
		return "", errors.New("host setup ran, but ignore_msrs is still disabled")
	}
	if h.checkKVM().Status != OK {
		return user + " was added to the kvm group: log out and back in (or reboot) so /dev/kvm opens.", nil
	}
	return "The host is ready for KVM.", nil
}

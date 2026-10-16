// Package core contém a lógica do app, sem dependência da UI.
package core

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// Version é uma versão do macOS aceita pelo fetch-macOS-v2.py (-s <Short>).
type Version struct {
	Short string
	Name  string
}

// Versions na ordem de lançamento; o índice decide o perfil de hardware.
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
	// validated: única versão testada de ponta a ponta no host de referência.
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

// Config é o que o usuário ajusta e o app persiste.
type Config struct {
	Workspace string `json:"workspace"`
	Version   string `json:"version"`
	RAMMiB    int    `json:"ram_mib"`
	Cores     int    `json:"cores"`
	VCPUs     int    `json:"vcpus"`
	SSHPort   int    `json:"ssh_port"`
}

func Defaults() Config {
	return Config{Workspace: "~/OSX-KVM", Version: validated, RAMMiB: 8192, Cores: 4, VCPUs: 8, SSHPort: 2222}
}

// Recommended ajusta CPU e RAM ao host. Sem GPU suportada o macOS desenha a tela
// na CPU, então mais vCPUs deixam o vídeo visivelmente mais fluido.
func Recommended(c Config, hostThreads, hostMemMiB int) Config {
	vcpus := min(max(hostThreads/2/2*2, 2), 16)
	c.VCPUs, c.Cores = vcpus, max(vcpus/2, 1)
	c.RAMMiB = min(max(hostMemMiB/4/1024*1024, 4096), 16384)
	return c
}

// FieldError aponta o campo inválido para a UI destacar.
type FieldError struct {
	Field string
	Msg   string
}

func (e *FieldError) Error() string { return e.Field + ": " + e.Msg }

// WorkspaceDir expande "~" e exige caminho absoluto.
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
		return "", &FieldError{"workspace", "precisa ser um caminho absoluto"}
	}
	return filepath.Clean(p), nil
}

func (c Config) Validate() error {
	if _, err := c.WorkspaceDir(); err != nil {
		return err
	}
	if versionIndex(c.Version) < 0 {
		return &FieldError{"version", "versão desconhecida: " + c.Version}
	}
	if c.RAMMiB < 4096 {
		return &FieldError{"ram", "mínimo de 4096 MiB"}
	}
	if c.Cores < 1 {
		return &FieldError{"cores", "mínimo de 1 núcleo"}
	}
	if c.VCPUs < 1 || c.VCPUs%c.Cores != 0 {
		return &FieldError{"vcpus", "precisa ser múltiplo do número de núcleos"}
	}
	if c.SSHPort < 1024 || c.SSHPort > 65535 {
		return &FieldError{"ssh_port", "use uma porta entre 1024 e 65535"}
	}
	return nil
}

// ConfigPath é ~/.config/macos-kvm-gui/config.json (ou equivalente XDG).
func ConfigPath() (string, error) {
	dir, err := os.UserConfigDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "macos-kvm-gui", "config.json"), nil
}

// LoadConfig devolve os padrões quando o arquivo ainda não existe.
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
		return Defaults(), fmt.Errorf("config inválida em %s: %w", path, err)
	}
	return c, nil
}

// SaveConfig valida e grava de forma atômica (temporário + rename).
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

// Profile é o hardware virtual derivado da versão do macOS.
type Profile struct {
	CPU          string
	NIC          string
	Experimental bool
	NeedsAVX2    bool
}

// cpuFlags vem do MY_OPTIONS do OpenCore-Boot.sh.
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

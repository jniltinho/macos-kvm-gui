package core

import (
	"errors"
	"fmt"
	"slices"
	"strconv"
	"strings"
)

// ConfigKeys are the keys of `config set` and the MCP server's vm_config_set, and how each one
// is applied. nic<N>.<field> keys edit network adapter N (1-4).
var ConfigKeys = map[string]func(*Config, string) error{
	"workspace":  func(c *Config, v string) error { c.Workspace = v; return nil },
	"version":    func(c *Config, v string) error { c.Version = v; return nil },
	"display":    func(c *Config, v string) error { c.Display = v; return nil },
	"gpu":        func(c *Config, v string) error { c.GPU = v; return nil },
	"graphics":   func(c *Config, v string) error { c.Graphics = v; return nil },
	"resolution": func(c *Config, v string) error { c.Resolution = v; return nil },
	"ram":        intKey(func(c *Config, n int) { c.RAMMiB = n }),
	"cores":      intKey(func(c *Config, n int) { c.Cores = n }),
	"vcpus":      intKey(func(c *Config, n int) { c.VCPUs = n }),
	"ssh-port":   intKey(func(c *Config, n int) { c.SSHPort = n }),
	"root-patch": func(c *Config, v string) error {
		b, err := strconv.ParseBool(v)
		if err != nil {
			return fmt.Errorf("%q is not true or false", v)
		}
		c.RootPatch = b
		return nil
	},
}

func init() {
	fields := map[string]func(*NIC, string) error{
		"enabled": func(n *NIC, v string) error {
			b, err := strconv.ParseBool(v)
			if err != nil {
				return fmt.Errorf("%q is not true or false", v)
			}
			n.Enabled = b
			return nil
		},
		"mode":   func(n *NIC, v string) error { n.Mode = v; return nil },
		"bridge": func(n *NIC, v string) error { n.Bridge = v; return nil },
		"model":  func(n *NIC, v string) error { n.Model = v; return nil },
		"mac": func(n *NIC, v string) error {
			if v == "random" {
				v = RandomMAC()
			}
			n.MAC = strings.ToLower(v)
			return nil
		},
	}
	for i := 1; i <= MaxNICs; i++ {
		for name, set := range fields {
			ConfigKeys[fmt.Sprintf("nic%d.%s", i, name)] = func(c *Config, v string) error {
				n, err := c.NIC(i - 1)
				if err != nil {
					return err
				}
				return set(n, v)
			}
		}
	}
}

func intKey(set func(*Config, int)) func(*Config, string) error {
	return func(c *Config, v string) error {
		n, err := strconv.Atoi(v)
		if err != nil {
			return fmt.Errorf("%q is not a whole number", v)
		}
		set(c, n)
		return nil
	}
}

// AgentConfigKeys are the keys agents may set (MCP vm_config_set). root-patch needs a
// confirmation and downloads, workspace is a folder change: those stay with the app and the CLI.
var AgentConfigKeys = []string{"ssh-port", "ram", "cores", "vcpus", "graphics", "resolution", "display", "gpu", "version"}

// ConfigWarnings are the warnings for saved settings: a shared SSH port, and a Reims resolution
// that crashes WindowServer before Tahoe.
func ConfigWarnings(e VMEntry) []string {
	var w []string
	if msg := SSHPortWarning(e.Config.SSHPort, e.ID); msg != "" {
		w = append(w, msg)
	}
	c := e.Config
	if c.Reims() && WideResolution(c.ReimsResolution()) && !c.ReimsKext() {
		w = append(w, "between 2048 and 3840 px wide, WindowServer crashed at every login on macOS Sequoia (ReimsVgpu.kext, which fixes it, loads on macOS 26 only); 3840x2160 works as a HiDPI desktop")
	}
	return w
}

// SetVMConfig applies agent settings (AgentConfigKeys) to a stopped VM. It holds the VM
// folder's lock, the one a start takes, and reloads, applies, validates and saves in one
// immediate transaction, so neither app can start or edit the VM meanwhile. It returns the saved
// VM and ConfigWarnings.
func SetVMConfig(e VMEntry, values map[string]string) (VMEntry, []string, error) {
	if len(values) == 0 {
		return e, nil, errors.New("no settings given")
	}
	for k := range values {
		if k == "root-patch" || k == "workspace" || strings.HasPrefix(k, "nic") {
			return e, nil, fmt.Errorf("%s: use the app or `config set`", k)
		}
		if !slices.Contains(AgentConfigKeys, k) {
			return e, nil, fmt.Errorf("unknown key %q: use %s", k, strings.Join(AgentConfigKeys, ", "))
		}
	}
	dir, err := e.Config.WorkspaceDir()
	if err != nil {
		return e, nil, err
	}
	unlock, err := NewWorkspace(dir).Lock()
	if errors.Is(err, ErrBusy) {
		return e, nil, errors.New("the VM is starting or its folder is busy (another operation or app): try again when it is done")
	}
	if err != nil {
		return e, nil, err
	}
	defer unlock()
	if _, ok := FindRunning(e.Config); ok {
		return e, nil, errors.New("the VM is running: shut the VM down first")
	}
	db, err := openDB()
	if err != nil {
		return e, nil, err
	}
	defer db.Close()
	tx, err := db.Begin()
	if err != nil {
		return e, nil, err
	}
	defer tx.Rollback()
	cur, err := scanVM(tx.QueryRow(`SELECT `+vmColumns+` FROM vms WHERE id = ?`, e.ID))
	if err != nil {
		return e, nil, fmt.Errorf("VM %s: %w", e.ID, err)
	}
	c := cur.Config
	keys := make([]string, 0, len(values))
	for k := range values {
		keys = append(keys, k)
	}
	slices.Sort(keys)
	for _, k := range keys {
		if err := ConfigKeys[k](&c, values[k]); err != nil {
			return e, nil, fmt.Errorf("%s: %w", k, err)
		}
	}
	if err := c.Validate(); err != nil {
		return e, nil, err
	}
	if err := updateVM(tx, cur.ID, c); err != nil {
		return e, nil, err
	}
	if err := tx.Commit(); err != nil {
		return e, nil, err
	}
	writeSnapshot(c)
	cur.Config = c
	return cur, ConfigWarnings(cur), nil
}

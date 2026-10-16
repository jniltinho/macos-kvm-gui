package cmd

import (
	"encoding/json"
	"errors"
	"fmt"
	"runtime"
	"strconv"
	"strings"

	"github.com/spf13/cobra"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

var configCmd = &cobra.Command{Use: "config", Short: "Show or change the VM settings"}

var configShowCmd = &cobra.Command{
	Use:   "show",
	Short: "Print the settings as JSON",
	RunE: func(cmd *cobra.Command, _ []string) error {
		e, err := currentVM()
		if err != nil {
			return err
		}
		b, _ := json.MarshalIndent(e.Config, "", "  ")
		src := e.Path
		if src == "" {
			db, _ := core.DBPath()
			src = db + " (VM " + e.ID + ")"
		}
		fmt.Fprintf(cmd.OutOrStdout(), "# %s\n%s\n", src, b)
		return nil
	},
}

// configKeys lists the settable keys and how each one is applied.
var configKeys = map[string]func(*core.Config, string) error{
	"workspace": func(c *core.Config, v string) error { c.Workspace = v; return nil },
	"version":   func(c *core.Config, v string) error { c.Version = v; return nil },
	"display":   func(c *core.Config, v string) error { c.Display = v; return nil },
	"gpu":       func(c *core.Config, v string) error { c.GPU = v; return nil },
	"ram":       intKey(func(c *core.Config, n int) { c.RAMMiB = n }),
	"cores":     intKey(func(c *core.Config, n int) { c.Cores = n }),
	"vcpus":     intKey(func(c *core.Config, n int) { c.VCPUs = n }),
	"ssh-port":  intKey(func(c *core.Config, n int) { c.SSHPort = n }),
	"root-patch": func(c *core.Config, v string) error {
		b, err := strconv.ParseBool(v)
		if err != nil {
			return fmt.Errorf("%q is not true or false", v)
		}
		c.RootPatch = b
		return nil
	},
}

var rootPatchReverted bool

// nic<N>.<field> keys edit network adapter N (1-4).
func init() {
	configSetCmd.Flags().BoolVar(&rootPatchReverted, "root-patch-reverted", false, "confirm the root patch was reverted in OCLP, to turn root-patch mode off")
	fields := map[string]func(*core.NIC, string) error{
		"enabled": func(n *core.NIC, v string) error {
			b, err := strconv.ParseBool(v)
			if err != nil {
				return fmt.Errorf("%q is not true or false", v)
			}
			n.Enabled = b
			return nil
		},
		"mode":   func(n *core.NIC, v string) error { n.Mode = v; return nil },
		"bridge": func(n *core.NIC, v string) error { n.Bridge = v; return nil },
		"model":  func(n *core.NIC, v string) error { n.Model = v; return nil },
		"mac": func(n *core.NIC, v string) error {
			if v == "random" {
				v = core.RandomMAC()
			}
			n.MAC = strings.ToLower(v)
			return nil
		},
	}
	for i := 1; i <= core.MaxNICs; i++ {
		for name, set := range fields {
			i, set := i, set
			configKeys[fmt.Sprintf("nic%d.%s", i, name)] = func(c *core.Config, v string) error {
				n, err := c.NIC(i - 1)
				if err != nil {
					return err
				}
				return set(n, v)
			}
		}
	}
}

func intKey(set func(*core.Config, int)) func(*core.Config, string) error {
	return func(c *core.Config, v string) error {
		n, err := strconv.Atoi(v)
		if err != nil {
			return fmt.Errorf("%q is not a whole number", v)
		}
		set(c, n)
		return nil
	}
}

var configSetCmd = &cobra.Command{
	Use:   "set key=value...",
	Short: "Change settings (workspace, version, ram, cores, vcpus, ssh-port, display, gpu, root-patch, nic<N>.enabled|mode|bridge|model|mac)",
	Example: "  macos-kvm-gui config set version=sonoma ram=16384 cores=8 vcpus=16 display=app\n" +
		"  macos-kvm-gui config set nic2.enabled=true nic2.mode=bridged nic2.bridge=br0 nic2.model=vmxnet3 nic2.mac=random",
	Args: cobra.MinimumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		e, err := currentVM()
		if err != nil {
			return err
		}
		c := e.Config
		for _, a := range args {
			k, v, ok := strings.Cut(a, "=")
			set, known := configKeys[k]
			if !ok || !known {
				return fmt.Errorf("expected key=value with a known key, got %q", a)
			}
			if err := set(&c, v); err != nil {
				return fmt.Errorf("%s: %w", k, err)
			}
		}
		out := cmd.OutOrStdout()
		switch {
		case c.RootPatch && (!e.Config.RootPatch || !core.RootPatchKextsReady()):
			// Also when the mode was already on but the kexts are missing (an imported VM, a
			// moved Workspace).
			if err := c.Validate(); err != nil {
				return err
			}
			fmt.Fprintln(out, "root-patch mode lowers SIP to 0x803 and loads AMFIPass, RSRHelper, RestrictEvents and KDKlessWorkaround;")
			fmt.Fprintln(out, "downloading them from OpenCore Legacy Patcher 2.5.1 (checked against their SHA-256)...")
			if err := core.DownloadRootPatchKexts(cmd.Context()); err != nil {
				return err
			}
		case !c.RootPatch && e.Config.RootPatch && !rootPatchReverted:
			return errors.New("revert the root patch in OCLP inside macOS first (Post-Install Root Patch › Revert Root Patches) and restart: " +
				"remaining root patches can prevent macOS from booting once its protections are restored. Then run this again with --root-patch-reverted")
		}
		if err := core.SaveVM(e, c); err != nil {
			return err
		}
		fmt.Fprintln(out, "saved", e.DisplayName())
		return nil
	},
}

var configRecommendCmd = &cobra.Command{
	Use:   "recommend",
	Short: "Set CPU and RAM to the values recommended for this host",
	RunE: func(cmd *cobra.Command, _ []string) error {
		e, err := currentVM()
		if err != nil {
			return err
		}
		c := core.Recommended(e.Config, runtime.NumCPU(), core.SystemHost().MemTotalMiB())
		if err := core.SaveVM(e, c); err != nil {
			return err
		}
		fmt.Fprintf(cmd.OutOrStdout(), "saved %s: %d MiB, %d vCPUs (%d cores)\n", e.DisplayName(), c.RAMMiB, c.VCPUs, c.Cores)
		return nil
	},
}

func init() {
	configCmd.AddCommand(configShowCmd, configSetCmd, configRecommendCmd)
	rootCmd.AddCommand(configCmd)
}

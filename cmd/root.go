// Package cmd is the command line: one file per command, built with Cobra.
package cmd

import (
	"fmt"
	"os"

	"github.com/spf13/cobra"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
	"github.com/jniltinho/macos-kvm-gui/internal/gui"
)

var (
	version = "dev"
	cfgFile string
	vmName  string
)

var rootCmd = &cobra.Command{
	Use:   "macos-kvm-gui",
	Short: "Install and run macOS on Linux KVM (home lab use)",
	Long: `macOS KVM installs and runs macOS on Linux KVM, with OpenCore and OVMF built in.
Without a command it opens the desktop app; the commands below automate the same steps.

For home lab use: macOS's license only allows it on Apple-branded hardware.`,
	SilenceUsage: true,
	RunE: func(*cobra.Command, []string) error {
		gui.Run(version, cfgFile) // "" = the VM list; --config = one VM file
		return nil
	},
}

// Execute runs the command line; appVersion comes from the build.
func Execute(appVersion string) {
	version, rootCmd.Version = appVersion, appVersion
	if err := rootCmd.Execute(); err != nil {
		os.Exit(1)
	}
}

func init() {
	rootCmd.PersistentFlags().StringVar(&cfgFile, "config", "", "use this VM settings file instead of the VM list")
	rootCmd.PersistentFlags().StringVar(&vmName, "vm", "", "VM to act on, by name or id (default: the selected VM)")
}

// currentVM resolves which settings file the command acts on: --config, then --vm, then the
// selected VM. With no VM yet, it creates "macOS Sonoma" with the defaults, like the app.
func currentVM() (core.VMEntry, error) {
	if cfgFile != "" {
		c, err := core.LoadConfig(cfgFile)
		return core.VMEntry{ID: "config", Path: cfgFile, Config: c}, err
	}
	vms, err := core.ListVMs()
	if err != nil {
		return core.VMEntry{}, err
	}
	if len(vms) == 0 {
		c := core.Defaults()
		c.Name = "macOS Sonoma"
		c.Workspace = core.VMFolder(c.Name)
		c.Networks = core.NewNetworks(c.Version)
		e, err := core.CreateVM(c)
		return e, err
	}
	if vmName != "" {
		if e, ok := core.FindVM(vms, vmName); ok {
			return e, nil
		}
		return core.VMEntry{}, fmt.Errorf("no VM named %q (see `vm list`)", vmName)
	}
	e, _ := core.SelectedVM(vms)
	return e, nil
}

// loadConfig returns the current VM's settings and the file they live in.
func loadConfig() (core.Config, string, error) {
	e, err := currentVM()
	return e.Config, e.Path, err
}

func loadWorkspace() (core.Config, core.Workspace, error) {
	c, _, err := loadConfig()
	if err != nil {
		return c, core.Workspace{}, err
	}
	dir, err := c.WorkspaceDir()
	return c, core.NewWorkspace(dir), err
}

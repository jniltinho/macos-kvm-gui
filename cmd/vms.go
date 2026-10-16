package cmd

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"text/tabwriter"

	"github.com/spf13/cobra"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

var (
	createVersion, createWorkspace string
	deletePurge, deleteYes         bool
)

var vmListCmd = &cobra.Command{
	Use:   "list",
	Short: "List the VMs (the selected one is marked with *)",
	RunE: func(cmd *cobra.Command, _ []string) error {
		vms, err := core.ListVMs()
		if err != nil {
			return err
		}
		sel, _ := core.SelectedVM(vms)
		tw := tabwriter.NewWriter(cmd.OutOrStdout(), 0, 4, 2, ' ', 0)
		fmt.Fprintln(tw, "\tNAME\tSTATUS\tVERSION\tRAM\tVCPUS\tFOLDER")
		for _, e := range vms {
			mark := ""
			if e.ID == sel.ID {
				mark = "*"
			}
			c := e.Config
			fmt.Fprintf(tw, "%s\t%s\t%s\t%s\t%d MiB\t%d\t%s\n", mark, e.DisplayName(), statusOf(c), c.Version, c.RAMMiB, c.VCPUs, c.Workspace)
		}
		return tw.Flush()
	},
}

var vmCreateCmd = &cobra.Command{
	Use:     "create <name>",
	Short:   "Create a VM with CPU and RAM recommended for this host, select it and prepare its folder",
	Example: "  macos-kvm-gui vm create \"Tahoe lab\" --version tahoe --folder ~/macOS-VMs/tahoe",
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		c := core.Recommended(core.Defaults(), runtime.NumCPU(), core.SystemHost().MemTotalMiB())
		c.Name, c.Version, c.Workspace = args[0], createVersion, createWorkspace
		if c.Workspace == "" {
			c.Workspace = core.VMFolder(c.Name)
		}
		c.Networks = core.NewNetworks(c.Version)
		if err := c.Validate(); err != nil {
			return err
		}
		e, err := core.CreateVM(c)
		if err != nil {
			return err
		}
		if err := core.SelectVM(e.ID); err != nil {
			return err
		}
		dir, _ := c.WorkspaceDir()
		if err := core.NewWorkspace(dir).Prepare(c); err != nil {
			return err
		}
		fmt.Fprintf(cmd.OutOrStdout(), "created and selected %s (%s), folder %s\n", e.DisplayName(), e.ID, dir)
		return nil
	},
}

var vmSelectCmd = &cobra.Command{
	Use:   "select <name>",
	Short: "Select the VM that commands (and the app) act on",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		vms, err := core.ListVMs()
		if err != nil {
			return err
		}
		e, ok := core.FindVM(vms, args[0])
		if !ok {
			return fmt.Errorf("no VM named %q (see `vm list`)", args[0])
		}
		if err := core.SelectVM(e.ID); err != nil {
			return err
		}
		fmt.Fprintln(cmd.OutOrStdout(), "selected", e.DisplayName())
		return nil
	},
}

var vmDeleteCmd = &cobra.Command{
	Use:   "delete <name>",
	Short: "Remove a VM from the list; --purge also deletes its disk, media and boot files",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		if !deleteYes {
			return errors.New("pass --yes to confirm")
		}
		vms, err := core.ListVMs()
		if err != nil {
			return err
		}
		e, ok := core.FindVM(vms, args[0])
		if !ok {
			return fmt.Errorf("no VM named %q (see `vm list`)", args[0])
		}
		removed, err := core.DeleteVM(e, deletePurge)
		for _, r := range removed {
			fmt.Fprintln(cmd.OutOrStdout(), "deleted", r)
		}
		return err
	},
}

var exportFull bool

var vmExportCmd = &cobra.Command{
	Use:     "export <name> [file]",
	Short:   "Write a VM's settings as JSON, or with --full a .tar bundle with everything it needs",
	Example: "  macos-kvm-gui vm export \"macOS Sonoma\" sonoma.json\n  macos-kvm-gui vm export \"macOS Sonoma\" sonoma.tar --full",
	Args:    cobra.RangeArgs(1, 2),
	RunE: func(cmd *cobra.Command, args []string) error {
		vms, err := core.ListVMs()
		if err != nil {
			return err
		}
		e, ok := core.FindVM(vms, args[0])
		if !ok {
			return fmt.Errorf("no VM named %q (see `vm list`)", args[0])
		}
		if exportFull {
			if len(args) != 2 {
				return errors.New("--full needs a destination file")
			}
			return core.ExportBundle(cmd.Context(), e, args[1], func(done, total int64) {
				fmt.Fprintf(cmd.ErrOrStderr(), "\rexporting: %.0f%%", float64(done)*100/float64(max(total, 1)))
			})
		}
		b, err := core.ExportVM(e)
		if err != nil {
			return err
		}
		if len(args) == 2 {
			return os.WriteFile(args[1], append(b, '\n'), 0o644)
		}
		fmt.Fprintln(cmd.OutOrStdout(), string(b))
		return nil
	},
}

var vmMoveCmd = &cobra.Command{
	Use:   "move <name> [folder]",
	Short: "Move a VM's disk, NVRAM and boot files to a new folder (default <Workspace>/<name>)",
	Args:  cobra.RangeArgs(1, 2),
	RunE: func(cmd *cobra.Command, args []string) error {
		vms, err := core.ListVMs()
		if err != nil {
			return err
		}
		e, ok := core.FindVM(vms, args[0])
		if !ok {
			return fmt.Errorf("no VM named %q (see `vm list`)", args[0])
		}
		dst := core.VMFolder(e.Config.Name)
		if len(args) == 2 {
			dst = args[1]
		}
		if e, err = core.MoveVM(e, dst); err != nil {
			return err
		}
		fmt.Fprintf(cmd.OutOrStdout(), "moved %s to %s\n", e.DisplayName(), e.Config.Workspace)
		return nil
	},
}

var importWorkspace string

var vmImportCmd = &cobra.Command{
	Use:   "import <file.json|file.tar>",
	Short: "Add a VM from a settings file or a full bundle (unpacked into --folder)",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		var e core.VMEntry
		var err error
		if strings.HasSuffix(strings.ToLower(args[0]), ".tar") {
			ws := importWorkspace
			if ws == "" {
				ws = core.VMFolder(strings.TrimSuffix(filepath.Base(args[0]), ".tar"))
			}
			dir, derr := (core.Config{Workspace: ws}).WorkspaceDir()
			if derr != nil {
				return derr
			}
			e, err = core.ImportBundle(cmd.Context(), args[0], dir, func(done, total int64) {
				fmt.Fprintf(cmd.ErrOrStderr(), "\rimporting: %.0f%%", float64(done)*100/float64(max(total, 1)))
			})
			fmt.Fprintln(cmd.ErrOrStderr())
		} else {
			e, err = core.ImportSettings(args[0])
		}
		if err != nil {
			return err
		}
		fmt.Fprintf(cmd.OutOrStdout(), "imported %s (%s), folder %s\n", e.DisplayName(), e.ID, e.Config.Workspace)
		return nil
	},
}

// statusOf is "running (pid N, up 1h2m)" or "stopped"; VMs started outside the app say so.
func statusOf(c core.Config) string {
	r, ok := core.FindRunning(c)
	if !ok {
		return "stopped"
	}
	s := fmt.Sprintf("running (pid %d, up %s)", r.PID, r.Uptime())
	if !r.ByApp {
		s += ", outside the app"
	}
	return s
}

var vmStatusCmd = &cobra.Command{
	Use:   "status [name]",
	Short: "Show whether a VM (default: the selected one) is running, and how to reach it",
	Args:  cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		e, err := currentVM()
		if len(args) == 1 {
			vms, lerr := core.ListVMs()
			if lerr != nil {
				return lerr
			}
			var ok bool
			if e, ok = core.FindVM(vms, args[0]); !ok {
				return fmt.Errorf("no VM named %q (see `vm list`)", args[0])
			}
		} else if err != nil {
			return err
		}
		out := cmd.OutOrStdout()
		c := e.Config
		fmt.Fprintf(out, "name:      %s (%s)\n", e.DisplayName(), e.ID)
		fmt.Fprintf(out, "status:    %s\n", statusOf(c))
		fmt.Fprintf(out, "macOS:     %s, %d MiB, %d vCPUs\n", c.Version, c.RAMMiB, c.VCPUs)
		fmt.Fprintf(out, "folder: %s\n", c.Workspace)
		if c.SSHForwarded() {
			fmt.Fprintf(out, "ssh:       ssh -p %d user@127.0.0.1\n", c.SSHPort)
		} else {
			fmt.Fprintln(out, "ssh:       no 127.0.0.1 forward (no NAT adapter): use the VM's address on its network")
		}
		if r, ok := core.FindRunning(c); ok && r.VNCSocket != "" {
			fmt.Fprintf(out, "screen:    VNC socket %s\n", r.VNCSocket)
		}
		return nil
	},
}

func init() {
	vmExportCmd.Flags().BoolVar(&exportFull, "full", false, "write a .tar bundle with the disk, NVRAM, identity and install media")
	vmImportCmd.Flags().StringVar(&importWorkspace, "folder", "", "VM folder for a .tar bundle (default <Workspace>/<name>)")
	vmCreateCmd.Flags().StringVar(&createVersion, "version", "sonoma", "macOS version (high-sierra ... tahoe)")
	vmCreateCmd.Flags().StringVar(&createWorkspace, "folder", "", "VM folder (default <Workspace>/<name>)")
	vmDeleteCmd.Flags().BoolVar(&deletePurge, "purge", false, "also delete the disk, install media and boot files the app created in the VM folder")
	vmDeleteCmd.Flags().BoolVar(&deleteYes, "yes", false, "confirm")
	vmCmd.AddCommand(vmListCmd, vmStatusCmd, vmCreateCmd, vmSelectCmd, vmDeleteCmd, vmExportCmd, vmImportCmd, vmMoveCmd)
}

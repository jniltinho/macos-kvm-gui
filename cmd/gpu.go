package cmd

import (
	"fmt"
	"text/tabwriter"

	"github.com/spf13/cobra"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

var gpuCmd = &cobra.Command{Use: "gpu", Short: "GPU passthrough (experimental, untested on real hardware)"}

var gpuListCmd = &cobra.Command{
	Use:   "list",
	Short: "List display adapters and whether they can be passed to the VM",
	Run: func(cmd *cobra.Command, _ []string) {
		tw := tabwriter.NewWriter(cmd.OutOrStdout(), 0, 4, 2, ' ', 0)
		fmt.Fprintln(tw, "ADDRESS\tID\tNAME\tELIGIBLE\tREASON")
		for _, g := range core.SystemHost().GPUs() {
			fmt.Fprintf(tw, "%s\t%s\t%s\t%v\t%s\n", g.Addr, g.ID, g.Name, g.Eligible, g.Reason)
		}
		tw.Flush()
	},
}

var gpuReserveCmd = &cobra.Command{
	Use:   "reserve <pci-address>",
	Short: "Reserve an eligible GPU for vfio-pci (needs a host reboot)",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		h := core.SystemHost()
		for _, g := range h.GPUs() {
			if g.Addr == args[0] {
				if err := h.ReserveGPU(g); err != nil {
					return err
				}
				fmt.Fprintln(cmd.OutOrStdout(), "reserved", g.Addr, "for vfio-pci: reboot the host, then `config set gpu="+g.Addr+"`")
				return nil
			}
		}
		return fmt.Errorf("no GPU at %s (see `gpu list`)", args[0])
	},
}

var gpuReleaseCmd = &cobra.Command{
	Use:   "release",
	Short: "Remove the vfio-pci reservation (needs a host reboot)",
	RunE: func(cmd *cobra.Command, _ []string) error {
		if err := core.SystemHost().ReleaseGPU(); err != nil {
			return err
		}
		fmt.Fprintln(cmd.OutOrStdout(), "reservation removed: reboot the host")
		return nil
	},
}

func init() {
	gpuCmd.AddCommand(gpuListCmd, gpuReserveCmd, gpuReleaseCmd)
	rootCmd.AddCommand(gpuCmd)
}

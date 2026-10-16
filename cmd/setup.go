package cmd

import (
	"fmt"
	"os/user"

	"github.com/spf13/cobra"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

var setupCmd = &cobra.Command{
	Use:   "setup",
	Short: "Set up the host for KVM (asks for your password once)",
	Long: `Installs missing QEMU packages, loads the KVM module, adds you to the kvm group and
enables ignore_msrs, through pkexec. The libvirt and input groups are not needed.`,
	RunE: func(cmd *cobra.Command, _ []string) error {
		me, err := user.Current()
		if err != nil {
			return err
		}
		msg, err := core.SystemHost().SetupHost(me.Username)
		if err != nil {
			return err
		}
		fmt.Fprintln(cmd.OutOrStdout(), msg)
		return nil
	},
}

func init() { rootCmd.AddCommand(setupCmd) }

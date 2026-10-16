package cmd

import (
	"fmt"

	"github.com/spf13/cobra"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

var versionCmd = &cobra.Command{
	Use:   "version",
	Short: "Show the app version and the built-in boot files",
	Run: func(cmd *cobra.Command, _ []string) {
		fmt.Fprintln(cmd.OutOrStdout(), "macos-kvm-gui", version)
		fmt.Fprintln(cmd.OutOrStdout(), "boot files:", core.AssetVersions())
	},
}

func init() { rootCmd.AddCommand(versionCmd) }

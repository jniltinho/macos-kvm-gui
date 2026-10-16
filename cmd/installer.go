package cmd

import (
	"fmt"
	"text/tabwriter"

	"github.com/spf13/cobra"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

var installerCmd = &cobra.Command{Use: "installer", Short: "Manage the cached full macOS installers (download them with download --full)"}

var installerListCmd = &cobra.Command{
	Use:   "list",
	Short: "List the cached full installers",
	Args:  cobra.NoArgs,
	Run: func(cmd *cobra.Command, _ []string) {
		tw := tabwriter.NewWriter(cmd.OutOrStdout(), 0, 4, 2, ' ', 0)
		fmt.Fprintln(tw, "VERSION\tBUILD\tSIZE\tPATH")
		for _, c := range core.CachedInstallers() {
			fmt.Fprintf(tw, "%s\t%s\t%.1f GB\t%s\n", c.Version, c.Build, float64(c.Size)/1e9, c.Path)
		}
		tw.Flush()
	},
}

var installerRemoveCmd = &cobra.Command{
	Use:   "remove <build>",
	Short: "Delete a cached full installer",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		if err := core.RemoveInstaller(args[0]); err != nil {
			return err
		}
		fmt.Fprintln(cmd.OutOrStdout(), "removed", args[0])
		return nil
	},
}

func init() {
	installerCmd.AddCommand(installerListCmd, installerRemoveCmd)
	rootCmd.AddCommand(installerCmd)
}

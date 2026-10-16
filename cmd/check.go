package cmd

import (
	"fmt"
	"text/tabwriter"

	"github.com/spf13/cobra"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

var checkCmd = &cobra.Command{
	Use:   "check",
	Short: "Check that the host can run macOS on KVM",
	Long:  "Runs the Host tab's checks. Exits with an error when any check fails, so scripts can stop early.",
	RunE: func(cmd *cobra.Command, _ []string) error {
		c, _, err := loadConfig()
		if err != nil {
			return err
		}
		tw := tabwriter.NewWriter(cmd.OutOrStdout(), 0, 4, 2, ' ', 0)
		failed := 0
		for _, ch := range core.SystemHost().ChecksFor(c) {
			line := fmt.Sprintf("%s\t%s\t%s", ch.Status, ch.Name, ch.Detail)
			if ch.Hint != "" {
				line += " (" + ch.Hint + ")"
			}
			fmt.Fprintln(tw, line)
			if ch.Status == core.Fail {
				failed++
			}
		}
		tw.Flush()
		if failed > 0 {
			return fmt.Errorf("%d check(s) failed; `macos-kvm-gui setup` fixes the common ones", failed)
		}
		return nil
	},
}

func init() { rootCmd.AddCommand(checkCmd) }

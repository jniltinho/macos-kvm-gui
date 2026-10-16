package cmd

import (
	"fmt"

	"github.com/spf13/cobra"
)

var diskSize int

var diskCmd = &cobra.Command{Use: "disk", Short: "Manage the macOS disk"}

var diskCreateCmd = &cobra.Command{
	Use:   "create",
	Short: "Create the thin qcow2 disk (never overwrites an existing one)",
	RunE: func(cmd *cobra.Command, _ []string) error {
		_, w, err := loadWorkspace()
		if err != nil {
			return err
		}
		warning, err := w.CreateDisk(cmd.Context(), diskSize)
		if err != nil {
			return err
		}
		if warning != "" {
			fmt.Fprintln(cmd.ErrOrStderr(), "warning:", warning)
		}
		fmt.Fprintf(cmd.OutOrStdout(), "disk created: %d GiB\n", diskSize)
		return nil
	},
}

func init() {
	diskCreateCmd.Flags().IntVar(&diskSize, "size", 80, "virtual size in GiB (minimum 40)")
	diskCmd.AddCommand(diskCreateCmd)
	rootCmd.AddCommand(diskCmd)
}

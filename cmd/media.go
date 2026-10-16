package cmd

import (
	"errors"
	"fmt"

	"github.com/spf13/cobra"
)

var mediaYes bool

var mediaCmd = &cobra.Command{Use: "media", Short: "Manage the install media"}

var mediaRemoveCmd = &cobra.Command{
	Use:   "remove",
	Short: "Delete BaseSystem.img once macOS boots from the disk",
	RunE: func(cmd *cobra.Command, _ []string) error {
		if !mediaYes {
			return errors.New("pass --yes to confirm that macOS already boots from the installed disk")
		}
		_, w, err := loadWorkspace()
		if err != nil {
			return err
		}
		freed, err := w.RemoveMedia()
		if err != nil {
			return err
		}
		fmt.Fprintf(cmd.OutOrStdout(), "install media removed: %.2f GiB freed\n", float64(freed)/(1<<30))
		return nil
	},
}

func init() {
	mediaRemoveCmd.Flags().BoolVar(&mediaYes, "yes", false, "confirm that macOS boots from the disk")
	mediaCmd.AddCommand(mediaRemoveCmd)
	rootCmd.AddCommand(mediaCmd)
}

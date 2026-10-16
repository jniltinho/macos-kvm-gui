package cmd

import (
	"errors"
	"fmt"
	"strings"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
	"github.com/spf13/cobra"
)

var settingsYes bool

var settingsCmd = &cobra.Command{Use: "settings", Short: "Manage settings.yaml (theme, boot files, extra kexts)"}

var settingsShowCmd = &cobra.Command{
	Use:   "show",
	Short: "Show settings.yaml's path and the settings in effect",
	RunE: func(cmd *cobra.Command, _ []string) error {
		ks, err := core.LoadExtraKexts()
		if err != nil {
			return err
		}
		out := cmd.OutOrStdout()
		fmt.Fprintln(out, "file:", core.SettingsFile())
		fmt.Fprintln(out, "theme:", core.Setting("MKG_THEME"))
		fmt.Fprintln(out, "assets_dir:", core.Setting("MKG_ASSETS_DIR"))
		for _, e := range ks.Entries {
			fmt.Fprintf(out, "kext: %s", e.URL)
			if len(e.Kexts) > 0 {
				fmt.Fprintf(out, " (load %s)", strings.Join(e.Kexts, ", "))
			}
			fmt.Fprintln(out)
		}
		if len(ks.Disabled) > 0 {
			fmt.Fprintln(out, "disable:", strings.Join(ks.Disabled, ", "))
		}
		if len(ks.Entries) > 0 {
			fmt.Fprintln(out, "kexts downloaded:", ks.Ready())
		}
		return nil
	},
}

var settingsResetCmd = &cobra.Command{
	Use:   "reset",
	Short: "Overwrite settings.yaml with the default, which sets nothing",
	RunE: func(cmd *cobra.Command, _ []string) error {
		if !settingsYes {
			return errors.New("pass --yes to overwrite " + core.SettingsFile())
		}
		if err := core.ResetSettings(); err != nil {
			return err
		}
		fmt.Fprintln(cmd.OutOrStdout(), "settings reset:", core.SettingsFile())
		return nil
	},
}

func init() {
	settingsResetCmd.Flags().BoolVar(&settingsYes, "yes", false, "confirm overwriting settings.yaml")
	settingsCmd.AddCommand(settingsShowCmd, settingsResetCmd)
	rootCmd.AddCommand(settingsCmd)
}

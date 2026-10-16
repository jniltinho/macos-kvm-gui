package cmd

import (
	"fmt"
	"time"

	"github.com/spf13/cobra"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

var downloadVersion string

var downloadCmd = &cobra.Command{
	Use:   "download",
	Short: "Download, verify and convert the macOS recovery into the workspace",
	RunE: func(cmd *cobra.Command, _ []string) error {
		c, w, err := loadWorkspace()
		if err != nil {
			return err
		}
		version := c.Version
		if downloadVersion != "" {
			version = downloadVersion
		}
		out := cmd.OutOrStdout()
		var last time.Time
		stage := ""
		progress := func(p core.Progress) {
			if p.Stage != stage {
				stage = p.Stage
				fmt.Fprintf(out, "\n%s", stage)
			}
			if p.Pct >= 0 && time.Since(last) > 500*time.Millisecond {
				last = time.Now()
				fmt.Fprintf(out, "\r%s: %.1f%%", stage, p.Pct)
			}
		}
		if err := w.DownloadMedia(cmd.Context(), version, progress, func(s string) { fmt.Fprintln(out, s) }); err != nil {
			fmt.Fprintln(out)
			return err
		}
		fmt.Fprintf(out, "\ninstall media ready: %s (%s)\n", w.Path(core.MediaFile), version)
		return nil
	},
}

func init() {
	downloadCmd.Flags().StringVar(&downloadVersion, "version", "", "macOS version (default: the configured one), e.g. sonoma")
	rootCmd.AddCommand(downloadCmd)
}

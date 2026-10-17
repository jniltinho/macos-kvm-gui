package cmd

import (
	"fmt"

	"github.com/spf13/cobra"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

var reimsRelease string

var reimsCmd = &cobra.Command{Use: "reims", Short: "Install, build and inspect Reims vGPU's QEMU (experimental)"}

var reimsInstallCmd = &cobra.Command{
	Use:   "install",
	Short: "Download the prebuilt Reims vGPU QEMU for this host (Ubuntu 24.04/26.04, Debian 13, Rocky Linux 10) and activate it",
	RunE: func(cmd *cobra.Command, _ []string) error {
		out := cmd.OutOrStdout()
		return core.InstallReims(cmd.Context(), func(s string) { fmt.Fprintln(out, s) })
	},
}

var reimsBuildCmd = &cobra.Command{
	Use:   "build",
	Short: "Build Reims vGPU's QEMU from source in Docker and activate it",
	Long: `Build Reims vGPU's QEMU from source, at the commit the app pins, in an Ubuntu container,
caching the build in <Workspace>/.reims. A build for another release than the host's stays
staged, not activated.`,
	RunE: func(cmd *cobra.Command, _ []string) error {
		out := cmd.OutOrStdout()
		return core.BuildReims(cmd.Context(), reimsRelease, func(s string) { fmt.Fprintln(out, s) })
	},
}

var reimsStatusCmd = &cobra.Command{
	Use:   "status",
	Short: "Show the Reims vGPU QEMU a reims-vgpu VM runs",
	RunE: func(cmd *cobra.Command, _ []string) error {
		info, err := core.ReimsInstalled()
		if err != nil {
			return err
		}
		out := cmd.OutOrStdout()
		fmt.Fprintln(out, "qemu:", info.QEMU)
		fmt.Fprintln(out, "build:", info.Build)
		fmt.Fprintln(out, "commit:", info.Commit)
		if info.AppleGFX != nil {
			fmt.Fprintln(out, "apple-gfx-pci: no:", info.AppleGFX)
		} else {
			fmt.Fprintln(out, "apple-gfx-pci: yes")
		}
		return nil
	},
}

func init() {
	reimsBuildCmd.Flags().StringVar(&reimsRelease, "release", "", "Ubuntu release to build for, 24.04 or 26.04 (default: the host's)")
	reimsCmd.AddCommand(reimsInstallCmd, reimsBuildCmd, reimsStatusCmd)
	rootCmd.AddCommand(reimsCmd)
}

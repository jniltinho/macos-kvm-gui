package cmd

import "github.com/spf13/cobra"

var guiCmd = &cobra.Command{
	Use:   "gui",
	Short: "Open the desktop app (the default without a command)",
	RunE:  rootCmd.RunE,
}

func init() { rootCmd.AddCommand(guiCmd) }

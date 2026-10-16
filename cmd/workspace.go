package cmd

import (
	"fmt"

	"github.com/jniltinho/macos-kvm-gui/internal/core"

	"github.com/spf13/cobra"
)

var workspaceCmd = &cobra.Command{Use: "workspace", Short: "Show or set the Workspace (one folder per VM inside it), or create the selected VM's folder"}

var workspaceShowCmd = &cobra.Command{
	Use:   "show",
	Short: "Print the Workspace",
	Args:  cobra.NoArgs,
	Run:   func(cmd *cobra.Command, _ []string) { fmt.Fprintln(cmd.OutOrStdout(), core.WorkspaceRoot()) },
}

var workspaceSetCmd = &cobra.Command{
	Use:   "set <dir>",
	Short: "Set the Workspace; new and imported VMs get <dir>/<name>. Existing VMs are not moved",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		if err := core.SetWorkspaceRoot(args[0]); err != nil {
			return err
		}
		fmt.Fprintln(cmd.OutOrStdout(), "workspace:", core.WorkspaceRoot())
		return nil
	},
}

var workspaceCreateCmd = &cobra.Command{
	Use:   "create",
	Short: "Create the selected VM's folder and extract the built-in OpenCore and OVMF",
	RunE: func(cmd *cobra.Command, _ []string) error {
		c, w, err := loadWorkspace()
		if err != nil {
			return err
		}
		if err := w.Prepare(c); err != nil {
			return err
		}
		fmt.Fprintln(cmd.OutOrStdout(), "VM folder ready:", w.Dir)
		return nil
	},
}

func init() {
	workspaceCmd.AddCommand(workspaceShowCmd, workspaceSetCmd, workspaceCreateCmd)
	rootCmd.AddCommand(workspaceCmd)
}

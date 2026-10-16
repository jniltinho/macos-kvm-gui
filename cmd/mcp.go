package cmd

import (
	"context"

	"github.com/spf13/cobra"

	"github.com/jniltinho/macos-kvm-gui/internal/mcp"
)

var mcpCmd = &cobra.Command{
	Use:   "mcp",
	Short: "Run the MCP server for AI agents (stdio): screen, keys, pointer, start/stop, offline install",
	Long: `Runs a Model Context Protocol server on standard input and output, for AI agents such as
Claude Code or Codex. Register it once, for example:

  claude mcp add macos-kvm -- macos-kvm-gui mcp
  codex mcp add macos-kvm -- macos-kvm-gui mcp

It opens no network port. Agents can see a VM's screen, type and click in it, start and stop it
and install macOS from the offline installer; tools that lose data or accept Apple's license
need explicit flags, which only the user should decide.`,
	Args: cobra.NoArgs,
	RunE: func(*cobra.Command, []string) error {
		return mcp.Serve(context.Background(), version)
	},
}

func init() { rootCmd.AddCommand(mcpCmd) }

package cmd

import (
	"errors"
	"fmt"
	"os"
	"os/signal"
	"syscall"

	"github.com/spf13/cobra"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

var (
	vmDisplay string
	vmVerbose bool
	vmBootLog bool
)

var vmCmd = &cobra.Command{Use: "vm", Short: "Run the macOS VM"}

var vmStartCmd = &cobra.Command{
	Use:   "start",
	Short: "Start the VM and wait until it stops",
	Long: `Starts the VM in the foreground and returns when QEMU exits.
Ctrl+C asks macOS to shut down (ACPI); a second Ctrl+C forces the stop.
With display=app there is no window: the screen is on the printed private VNC socket.`,
	RunE: func(cmd *cobra.Command, _ []string) error {
		c, _, err := loadConfig()
		if err != nil {
			return err
		}
		if vmDisplay != "" {
			c.Display = vmDisplay
		}
		c.VerboseBoot = vmBootLog
		out := cmd.OutOrStdout()
		stopped := make(chan string, 1)
		vm := &core.VM{
			OnState: func(s core.VMState, msg string) {
				if msg != "" {
					fmt.Fprintln(out, "vm:", msg)
				}
				if s == core.Stopped {
					stopped <- msg
				}
			},
		}
		if vmVerbose {
			vm.OnLog = func(s string) { fmt.Fprintln(out, s) }
		}
		if err := vm.Start(c); err != nil {
			return err
		}
		fmt.Fprintln(out, "vm: running; Ctrl+C shuts it down")
		if sock := vm.VNCSocket(); sock != "" {
			fmt.Fprintln(out, "vm: screen on VNC socket", sock)
		}
		if line := vm.InstallerCommand(); line != "" {
			fmt.Fprintln(out, "vm: the full installer disc is attached; to install from it, type in the recovery's Terminal:", line)
		}
		sig := make(chan os.Signal, 2)
		signal.Notify(sig, os.Interrupt, syscall.SIGTERM)
		defer signal.Stop(sig)
		for asked := false; ; {
			select {
			case msg := <-stopped:
				if msg != "" && !asked {
					return errors.New(msg)
				}
				return nil
			case <-sig:
				if asked {
					fmt.Fprintln(out, "vm: forcing the stop")
					vm.Kill()
					continue
				}
				asked = true
				fmt.Fprintln(out, "vm: asking macOS to shut down (Ctrl+C again to force)")
				if err := vm.Powerdown(); err != nil {
					fmt.Fprintln(out, "vm:", err)
				}
			}
		}
	},
}

func init() {
	vmStartCmd.Flags().StringVar(&vmDisplay, "display", "", "app (VNC socket, no window) or window (QEMU window); default from config")
	vmStartCmd.Flags().BoolVarP(&vmVerbose, "verbose", "v", false, "print the QEMU command line and output")
	vmStartCmd.Flags().BoolVar(&vmBootLog, "verbose-boot", false, "show macOS's boot log on screen instead of the logo, for this start only (boot-args -v debug=0x100)")
	vmCmd.AddCommand(vmStartCmd)
	rootCmd.AddCommand(vmCmd)
}

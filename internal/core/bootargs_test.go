//go:build integration

package core

import (
	"os"
	"strings"
	"testing"
)

// TestPrintBootArgs prepares $MKG_BOOT_WS and prints the QEMU arguments, NUL-separated, to
// $MKG_BOOT_WS/args, so a real macOS boot can be run by hand against an overlay disk.
func TestPrintBootArgs(t *testing.T) {
	dir := os.Getenv("MKG_BOOT_WS")
	if dir == "" {
		t.Skip("MKG_BOOT_WS not set")
	}
	c := Defaults()
	c.Workspace, c.SSHPort = dir, 22299
	w := NewWorkspace(dir)
	if err := w.PrepareBoot(c, false); err != nil {
		t.Fatal(err)
	}
	args := BuildArgs(c, w, dir+"/qmp.sock", false, nil)
	if err := os.WriteFile(dir+"/args", []byte(strings.Join(args, "\x00")), 0o644); err != nil {
		t.Fatal(err)
	}
}

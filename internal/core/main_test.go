package core

import (
	"os"
	"testing"
)

// TestMain keeps every test away from the developer's real settings.yaml and .env.
func TestMain(m *testing.M) {
	dir, err := os.MkdirTemp("", "macos-kvm-gui-test-")
	if err != nil {
		panic(err)
	}
	os.Setenv("XDG_CONFIG_HOME", dir)
	code := m.Run()
	os.RemoveAll(dir)
	os.Exit(code)
}

// testDefaults is Defaults with an explicit SSH port, for tests that validate a config or build
// QEMU's arguments without creating a VM.
func testDefaults() Config {
	c := Defaults()
	c.SSHPort = FirstSSHPort
	return c
}

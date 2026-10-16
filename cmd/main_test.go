package cmd

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

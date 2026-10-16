package core

import (
	"os"
	"path/filepath"
	"strings"
)

// envFile is the optional local settings file: ~/.config/macos-kvm-gui/.env.
func envFile() string {
	dir, err := os.UserConfigDir()
	if err != nil {
		return ""
	}
	return filepath.Join(dir, "macos-kvm-gui", ".env")
}

// Setting returns key from the environment or, failing that, from the local .env file.
// The file holds plain KEY=value lines; # starts a comment and quotes around values are dropped.
func Setting(key string) string {
	if v, ok := os.LookupEnv(key); ok {
		return v
	}
	data, err := os.ReadFile(envFile())
	if err != nil {
		return ""
	}
	for _, line := range strings.Split(string(data), "\n") {
		k, v, ok := strings.Cut(strings.TrimSpace(line), "=")
		if !ok || strings.HasPrefix(k, "#") || strings.TrimSpace(k) != key {
			continue
		}
		return strings.Trim(strings.TrimSpace(v), `"'`)
	}
	return ""
}

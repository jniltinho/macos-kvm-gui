package core

import (
	"bytes"
	_ "embed"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"gopkg.in/yaml.v3"
)

//go:embed settings.default.yaml
var defaultSettingsYAML []byte

// fileSettings is settings.yaml, the user's settings file. The environment wins over it.
type fileSettings struct {
	Theme      string `yaml:"theme,omitempty"`      // MKG_THEME
	AssetsDir  string `yaml:"assets_dir,omitempty"` // MKG_ASSETS_DIR
	ReimsQEMU  string `yaml:"reims_qemu,omitempty"` // MKG_REIMS_QEMU
	ExtraKexts `yaml:",inline"`
}

// SettingsFile is ~/.config/macos-kvm-gui/settings.yaml ("" without a user config folder).
func SettingsFile() string {
	if env := envFile(); env != "" {
		return filepath.Join(filepath.Dir(env), "settings.yaml")
	}
	return ""
}

// loadSettingsFile reads settings.yaml, creating it from the commented default first.
func loadSettingsFile() (fileSettings, error) {
	var fs fileSettings
	data, err := os.ReadFile(SettingsFile())
	if errors.Is(err, os.ErrNotExist) && SettingsFile() != "" {
		data, err = defaultSettingsYAML, nil
		if os.MkdirAll(filepath.Dir(SettingsFile()), 0o755) == nil {
			writeFileAtomic(SettingsFile(), data, 0o644) // best effort: the default sets nothing
		}
	}
	if err != nil {
		return fs, err
	}
	dec := yaml.NewDecoder(bytes.NewReader(data))
	dec.KnownFields(true)
	if err := dec.Decode(&fs); err != nil && !errors.Is(err, io.EOF) {
		return fileSettings{}, fmt.Errorf("%s: %w", SettingsFile(), err)
	}
	var more yaml.Node
	if err := dec.Decode(&more); !errors.Is(err, io.EOF) {
		return fileSettings{}, fmt.Errorf("%s: only one YAML document is allowed (remove the --- separators)", SettingsFile())
	}
	return fs, nil
}

// ResetSettings overwrites settings.yaml with the commented default, which sets nothing.
func ResetSettings() error {
	if SettingsFile() == "" {
		return errors.New("no user config folder for settings.yaml")
	}
	if err := os.MkdirAll(filepath.Dir(SettingsFile()), 0o755); err != nil {
		return err
	}
	return writeFileAtomic(SettingsFile(), defaultSettingsYAML, 0o644)
}

// envFile is the optional local settings file: ~/.config/macos-kvm-gui/.env.
func envFile() string {
	dir, err := os.UserConfigDir()
	if err != nil {
		return ""
	}
	return filepath.Join(dir, "macos-kvm-gui", ".env")
}

// Setting returns key from the environment, then settings.yaml, then the older local .env file.
// The file holds plain KEY=value lines; # starts a comment and quotes around values are dropped.
func Setting(key string) string {
	if v, ok := os.LookupEnv(key); ok {
		return v
	}
	if fs, err := loadSettingsFile(); err == nil {
		if v := map[string]string{"MKG_THEME": fs.Theme, "MKG_ASSETS_DIR": fs.AssetsDir, "MKG_REIMS_QEMU": fs.ReimsQEMU}[key]; v != "" {
			return v
		}
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

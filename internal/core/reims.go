package core

import (
	"errors"
	"os"
	"path/filepath"
)

// Reims vGPU (github.com/steelbrain/reims-vgpu) is a paravirtual GPU for macOS guests that
// renders through the host's Vulkan driver. It runs on its own QEMU build, which the app
// installs from a prebuilt package or builds in Docker, but never embeds (docs/reims-vgpu.md).

// ReimsDir is <Workspace>/.reims, next to .kexts and .installers: qemu/<build>, qemu/current
// and the build caches.
func ReimsDir() (string, error) {
	dir, err := (Config{Workspace: WorkspaceRoot()}).WorkspaceDir()
	return filepath.Join(dir, ".reims"), err
}

// ReimsQEMU finds the Reims QEMU: reims_qemu in settings.yaml (or MKG_REIMS_QEMU), the app's
// own install in <Workspace>/.reims, then an upstream reims-ctl install.
func ReimsQEMU() (string, error) {
	if p := Setting("MKG_REIMS_QEMU"); p != "" {
		if _, err := os.Stat(p); err != nil {
			return "", errors.New("reims_qemu in " + SettingsFile() + ": " + err.Error())
		}
		return p, nil
	}
	var tried []string
	if dir, err := ReimsDir(); err == nil {
		tried = append(tried, filepath.Join(dir, "qemu", "current", "bin", "qemu-system-x86_64"))
	}
	data := os.Getenv("XDG_DATA_HOME")
	if !filepath.IsAbs(data) {
		home, _ := os.UserHomeDir()
		data = filepath.Join(home, ".local", "share")
	}
	tried = append(tried, filepath.Join(data, "reims-ctl", "qemu", "current", "bin", "qemu-system-x86_64"))
	for _, p := range tried {
		if st, err := os.Stat(p); err == nil && st.Mode().IsRegular() {
			return p, nil
		}
	}
	return "", errors.New("Reims vGPU is not installed (looked for " + tried[0] + "): install it in the Host tab or with `macos-kvm-gui reims install`")
}

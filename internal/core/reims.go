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
// own install in <Workspace>/.reims, then an upstream reims-ctl install. A relative reims_qemu
// is relative to settings.yaml's folder, a relative MKG_REIMS_QEMU to the current directory:
// the VM runs QEMU from its own folder, so the path must be absolute by then.
func ReimsQEMU() (string, error) {
	if p := Setting("MKG_REIMS_QEMU"); p != "" {
		if _, env := os.LookupEnv("MKG_REIMS_QEMU"); !env && !filepath.IsAbs(p) {
			p = filepath.Join(filepath.Dir(SettingsFile()), p)
		}
		p, _ = filepath.Abs(p) // the env's path, from the current directory
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

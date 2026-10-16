package core

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSetting(t *testing.T) {
	cfg := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", cfg)
	write(t, filepath.Join(cfg, "macos-kvm-gui", ".env"), "# comment\nMKG_FOO = \"from-file\"\nMKG_BAR=x\n")
	if got := Setting("MKG_FOO"); got != "from-file" {
		t.Errorf(".env value: %q", got)
	}
	t.Setenv("MKG_FOO", "from-env")
	if got := Setting("MKG_FOO"); got != "from-env" {
		t.Errorf("the environment must win: %q", got)
	}
	if got := Setting("MKG_MISSING"); got != "" {
		t.Errorf("missing key: %q", got)
	}
}

func TestAssetsDirOverride(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	if v := AssetVersions(); !strings.Contains(v, "OpenCore 1.") || !strings.Contains(v, "built in") {
		t.Errorf("embedded versions: %q", v)
	}
	// internal/assets has the layout MKG_ASSETS_DIR expects.
	dir, _ := filepath.Abs("../assets")
	t.Setenv("MKG_ASSETS_DIR", dir)
	if v := AssetVersions(); !strings.Contains(v, "from MKG_ASSETS_DIR="+dir) {
		t.Errorf("override not reported: %q", v)
	}
	w := readyWorkspace(t)
	if err := w.PrepareBoot(Defaults(), false); err != nil || !exists(w.Path(EFIDir+"/EFI/OC/OpenCore.efi")) {
		t.Fatalf("boot from override: %v", err)
	}
	t.Setenv("MKG_ASSETS_DIR", t.TempDir())
	if err := w.PrepareBoot(Defaults(), false); err == nil || !strings.Contains(err.Error(), "missing") {
		t.Fatalf("an empty override must be refused: %v", err)
	}
	// A copy without the firmware's fallback loader must be refused too.
	partial := t.TempDir()
	if err := os.CopyFS(partial, os.DirFS(dir)); err != nil {
		t.Fatal(err)
	}
	os.Remove(filepath.Join(partial, "files/EFI/BOOT/BOOTx64.efi"))
	t.Setenv("MKG_ASSETS_DIR", partial)
	if err := w.PrepareBoot(Defaults(), false); err == nil || !strings.Contains(err.Error(), "BOOTx64.efi") {
		t.Fatalf("an override without BOOTx64.efi must be refused: %v", err)
	}
}

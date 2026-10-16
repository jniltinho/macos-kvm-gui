package core

import (
	"crypto/rand"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"strings"

	"github.com/jniltinho/macos-kvm-gui/internal/assets"
)

// Boot files the app extracts into the workspace.
const (
	BootDir      = "macos-kvm-gui"
	EFIDir       = BootDir + "/efi" // exposed to QEMU as a FAT disk (vvfat)
	OVMFCode     = BootDir + "/OVMF_CODE_4M.fd"
	VarsFile     = BootDir + "/OVMF_VARS_4M.fd" // the VM's NVRAM: created once, never overwritten
	identityFile = BootDir + "/smbios.json"
	resolution   = "1920x1080@32"
	// scanInstalled limits OpenCore to APFS and HFS+ volumes (OC_SCAN_FILE_SYSTEM_LOCK |
	// OC_SCAN_ALLOW_FS_APFS | OC_SCAN_ALLOW_FS_HFS): it hides OpenCore's own FAT disk, so the
	// installed system is the default entry, and still finds HFS+ High Sierra installs.
	scanInstalled = 0x301
)

// identity is the VM's SMBIOS identity, generated once per workspace.
type identity struct {
	Serial string `json:"serial"`
	MLB    string `json:"mlb"`
	UUID   string `json:"uuid"`
	ROM    string `json:"rom"` // 6 bytes, hex
}

const serialChars = "0123456789ABCDEFGHJKLMNPQRSTUVWXYZ"

func randomString(n int) (string, error) {
	b := make([]byte, n)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	for i := range b {
		b[i] = serialChars[int(b[i])%len(serialChars)]
	}
	return string(b), nil
}

func newIdentity() (identity, error) {
	var id identity
	u := make([]byte, 16+6)
	if _, err := rand.Read(u); err != nil {
		return id, err
	}
	u[6] = u[6]&0x0f | 0x40 // UUID version 4
	u[8] = u[8]&0x3f | 0x80
	h := strings.ToUpper(hex.EncodeToString(u[:16]))
	id.UUID = h[:8] + "-" + h[8:12] + "-" + h[12:16] + "-" + h[16:20] + "-" + h[20:]
	id.ROM = hex.EncodeToString(u[16:])
	var err error
	if id.Serial, err = randomString(12); err != nil {
		return id, err
	}
	id.MLB, err = randomString(17)
	return id, err
}

func (w Workspace) loadIdentity() (identity, error) {
	var id identity
	data, err := os.ReadFile(w.Path(identityFile))
	if err == nil {
		err = json.Unmarshal(data, &id)
		return id, err
	}
	if !errors.Is(err, os.ErrNotExist) {
		return id, err
	}
	if id, err = newIdentity(); err != nil {
		return id, err
	}
	data, _ = json.MarshalIndent(id, "", "  ")
	return id, writeFileAtomic(w.Path(identityFile), data, 0o644)
}

// bootAssets returns the boot files: the ones embedded at build time or, when MKG_ASSETS_DIR
// is set (environment or local .env), a directory with the same layout as internal/assets.
func bootAssets() (fs.FS, error) {
	dir := Setting("MKG_ASSETS_DIR")
	if dir == "" {
		return assets.FS, nil
	}
	fsys := os.DirFS(dir)
	// Every file the embedded set has must be there (kexts, drivers, BOOTx64.efi, firmware...).
	err := fs.WalkDir(assets.FS, ".", func(p string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() || p == "files/VERSIONS" {
			return err
		}
		if st, err := fs.Stat(fsys, p); err != nil || st.IsDir() {
			return fmt.Errorf("MKG_ASSETS_DIR=%s is missing %s", dir, p)
		}
		return nil
	})
	return fsys, err
}

// AssetVersions describes the boot files in use, for the UI.
func AssetVersions() string {
	src := "built in"
	if dir := Setting("MKG_ASSETS_DIR"); dir != "" {
		src = "from MKG_ASSETS_DIR=" + dir
	}
	fsys, err := bootAssets()
	if err != nil {
		return err.Error()
	}
	v, err := fs.ReadFile(fsys, "files/VERSIONS")
	if err != nil {
		return "unknown versions, " + src
	}
	return strings.ReplaceAll(strings.TrimSpace(string(v)), "\n", ", ") + " (" + src + ")"
}

// SMBIOSModel is the Mac model the VM presents for a macOS version: one that supports the macOS version. iMac models also keep
// AppleMCEReporter (which panics on AMD hosts) from loading.
func SMBIOSModel(version string) string {
	switch version {
	case "high-sierra":
		return "iMac18,3"
	case "mojave":
		return "iMac19,1"
	}
	return "iMac20,1" // Catalina 10.15.6 to Tahoe
}

// OpenCoreConfig renders config.plist. With install media attached, the picker is shown and
// every file system is scanned so the installer can be chosen; otherwise OpenCore boots
// straight into the only APFS entry.
func OpenCoreConfig(c Config, id identity, withMedia bool) (string, error) {
	fsys, err := bootAssets()
	if err != nil {
		return "", err
	}
	tmpl, err := fs.ReadFile(fsys, "config.plist.tmpl")
	if err != nil {
		return "", err
	}
	rom, err := hex.DecodeString(id.ROM)
	if err != nil || len(rom) != 6 {
		return "", fmt.Errorf("invalid SMBIOS ROM %q", id.ROM)
	}
	picker, timeout, scan := "<false/>", "0", fmt.Sprint(scanInstalled)
	if withMedia {
		picker, timeout, scan = "<true/>", "5", "0"
	}
	bootArgs := "keepsyms=1"
	if c.VerboseBoot {
		bootArgs += " -v debug=0x100"
	}
	if c.GPU != "" {
		bootArgs += " agdpmod=pikera" // Navi cards show a black screen on iMac board IDs without it
	}
	return strings.NewReplacer(
		"@@SHOWPICKER@@", picker,
		"@@TIMEOUT@@", timeout,
		"@@SCANPOLICY@@", scan,
		"@@BOOTARGS@@", bootArgs,
		"@@PREVLANG@@", base64.StdEncoding.EncodeToString([]byte(RecoveryKeyboard())),
		"@@PRODUCT@@", SMBIOSModel(c.Version),
		"@@SERIAL@@", id.Serial,
		"@@MLB@@", id.MLB,
		"@@UUID@@", id.UUID,
		"@@ROM@@", base64.StdEncoding.EncodeToString(rom),
		"@@RESOLUTION@@", resolution,
	).Replace(string(tmpl)), nil
}

// PrepareBoot refreshes the embedded boot files and config.plist in the workspace.
// The NVRAM and the SMBIOS identity are created once and kept.
func (w Workspace) PrepareBoot(c Config, withMedia bool) error {
	fsys, err := bootAssets()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(w.Path(BootDir), 0o755); err != nil {
		return err
	}
	if err := os.RemoveAll(w.Path(EFIDir)); err != nil {
		return err
	}
	err = fs.WalkDir(fsys, "files/EFI", func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		dst := w.Path(EFIDir + strings.TrimPrefix(p, "files"))
		if d.IsDir() {
			return os.MkdirAll(dst, 0o755)
		}
		return copyAsset(fsys, p, dst)
	})
	if err != nil {
		return fmt.Errorf("extracting OpenCore: %w", err)
	}
	if err := copyAsset(fsys, "files/ovmf/OVMF_CODE_4M.fd", w.Path(OVMFCode)); err != nil {
		return err
	}
	if !exists(w.Path(VarsFile)) {
		// A half-written NVRAM would never be retried, so it is published only once complete.
		tmp := w.Path(VarsFile) + ".tmp"
		if err := copyAsset(fsys, "files/ovmf/OVMF_VARS_4M.fd", tmp); err != nil {
			os.Remove(tmp)
			return err
		}
		if err := os.Rename(tmp, w.Path(VarsFile)); err != nil {
			os.Remove(tmp)
			return err
		}
	}
	id, err := w.loadIdentity()
	if err != nil {
		return fmt.Errorf("SMBIOS identity: %w", err)
	}
	cfg, err := OpenCoreConfig(c, id, withMedia)
	if err != nil {
		return err
	}
	if err := os.WriteFile(w.Path(EFIDir+"/EFI/OC/config.plist"), []byte(cfg), 0o644); err != nil {
		return err
	}
	// OpenCore's own disk would show in its picker as "QEMU VVFAT", ahead of macOS Base System.
	// .contentVisibility "Disabled" hides that volume's entries only, so the recovery or the
	// installed system is the first, default entry. (A stricter ScanPolicy would hide it too,
	// but the file system lock stops the Sonoma recovery at the prohibited sign.)
	for _, p := range []string{EFIDir + "/.contentVisibility", EFIDir + "/EFI/BOOT/.contentVisibility"} {
		if err := os.WriteFile(w.Path(p), []byte("Disabled"), 0o644); err != nil {
			return err
		}
	}
	return nil
}

func copyAsset(fsys fs.FS, src, dst string) error {
	in, err := fsys.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()
	out, err := os.Create(dst)
	if err != nil {
		return err
	}
	if _, err := io.Copy(out, in); err != nil {
		out.Close()
		return err
	}
	return out.Close()
}

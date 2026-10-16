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
	"path"
	"path/filepath"
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
	var disabled []string
	if c.Extras != nil {
		disabled = c.Extras.Disabled
	}
	text, err := kextSwitches(string(tmpl), disabled)
	if err != nil {
		return "", err
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
	if c.RootPatch {
		bootArgs += rootPatchBootArgs
	}
	plist := rootPatchPlist(c.RootPatch, c.rootPatchWasOn)
	if c.reimsKext {
		plist["@@ROOTPATCHKEXTS@@"] += kextEntry("ReimsVgpu", reimsKextMinKernel, "ReimsVgpu.kext (Reims vGPU on macOS 26, from reims-util "+reimsUtilCommit[:7]+")")
	}
	for _, k := range c.extraKexts {
		plist["@@ROOTPATCHKEXTS@@"] += kextEntry(k[1], "", k[1]+".kext (settings.yaml)")
	}
	pairs := []string{}
	for k, v := range plist {
		pairs = append(pairs, k, v)
	}
	return strings.NewReplacer(append(pairs,
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
	)...).Replace(text), nil
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
	if c.RootPatch {
		if err := w.addRootPatchKexts(); err != nil {
			return err
		}
	}
	// Only at a start (Extras set), where the kext was downloaded first: creating a VM stays
	// offline, and config.plist is written again at every start.
	if c.ReimsKext() && c.Extras != nil {
		if err := w.addReimsKext(); err != nil {
			return err
		}
		c.reimsKext = true
	}
	if c.Extras != nil && len(c.Extras.Entries) > 0 {
		builtin, err := fs.Glob(fsys, "files/EFI/OC/Kexts/*.kext")
		if err != nil {
			return err
		}
		for i, b := range builtin {
			builtin[i] = strings.TrimSuffix(path.Base(b), ".kext")
		}
		if err := w.addExtraKexts(&c, builtin); err != nil {
			return err
		}
	}
	c.rootPatchWasOn = !c.RootPatch && exists(w.Path(rootPatchMarker))
	cfg, err := OpenCoreConfig(c, id, withMedia)
	if err != nil {
		return err
	}
	if err := os.WriteFile(w.Path(EFIDir+"/EFI/OC/config.plist"), []byte(cfg), 0o644); err != nil {
		return err
	}
	// While the marker is there, boots with the mode off delete the lowered SIP; it is removed
	// only after such a boot really ran (see ClearRootPatchMarker), not when a config is merely
	// written (workspace create, a QEMU that fails to start).
	if c.RootPatch {
		if err := os.WriteFile(w.Path(rootPatchMarker), []byte("on\n"), 0o644); err != nil {
			return err
		}
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

// addRootPatchKexts copies the cached OCLP kexts into the extracted EFI (see rootpatch.go).
func (w Workspace) addRootPatchKexts() error {
	if !RootPatchKextsReady() {
		return errors.New("root-patch mode needs OCLP's kexts: download them in the GPU tab (or config set root-patch=true)")
	}
	src, err := RootPatchKextsDir()
	if err != nil {
		return err
	}
	for _, k := range rootPatchKexts {
		if err := copyTree(filepath.Join(src, k.Name+".kext"), w.Path(EFIDir+"/EFI/OC/Kexts/"+k.Name+".kext")); err != nil {
			return fmt.Errorf("copying %s.kext: %w", k.Name, err)
		}
	}
	return nil
}

func copyTree(src, dst string) error {
	return filepath.WalkDir(src, func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, _ := filepath.Rel(src, p)
		target := filepath.Join(dst, rel)
		if d.IsDir() {
			return os.MkdirAll(target, 0o755)
		}
		if !d.Type().IsRegular() {
			return fmt.Errorf("unexpected file %s", p)
		}
		return copyAsset(os.DirFS(filepath.Dir(p)), filepath.Base(p), target)
	})
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

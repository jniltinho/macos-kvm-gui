package core

import (
	"archive/zip"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// Root-patch mode (experimental, per VM, off by default): OpenCore settings that let OpenCore
// Legacy Patcher (OCLP) root-patch a passed-through NVIDIA Kepler GPU inside Monterey to
// Sequoia, matching OCLP 2.5.1's own profile for Kepler: SIP lowered to 0x803, AMFIPass instead
// of disabling AMFI, RSRHelper with -nokcmismatchpanic, ipc_control_port_options=0,
// KDKlessWorkaround, and RestrictEvents blocking mediaanalysisd (revblock=media). The app never
// runs OCLP: the user root-patches inside macOS.

// RootPatchVersion reports whether root-patch mode applies to a macOS version: Big Sur drives
// Kepler natively, and Tahoe is left out until OCLP supports it in a stable release.
func RootPatchVersion(version string) bool {
	switch version {
	case "monterey", "ventura", "sonoma", "sequoia":
		return true
	}
	return false
}

// keplerIDs are OCLP 2.5.1's kepler_ids (datasets/pci_data.py), NVIDIA vendor 10de: an explicit
// list, never ranges or marketing names (a GT 730 can be Fermi, a GTX 750 is Maxwell).
var keplerIDs = map[string]bool{}

func init() {
	for _, id := range []string{
		"0fc0", "0fc1", "0fc2", "0fc5", "0fc6", "0fc8", "0fc9", "0fcd", "0fce", "0fd1", "0fd2", "0fd3",
		"0fd4", "0fd5", "0fd6", "0fd8", "0fd9", "0fdb", "0fdf", "0fe0", "0fe1", "0fe2", "0fe3", "0fe4",
		"0fe5", "0fe6", "0fe7", "0fe8", "0fe9", "0fea", "0fec", "0fed", "0fee", "0fef", "0ff1", "0ff2",
		"0ff3", "0ff5", "0ff6", "0ff7", "0ff8", "0ff9", "0ffa", "0ffb", "0ffc", "0ffd", "0ffe", "0fff",
		"1001", "1003", "1004", "1005", "1007", "1008", "100a", "100c", "101e", "101f", "1020", "1021",
		"1022", "1023", "1024", "1026", "1027", "1028", "1029", "102a", "102d", "102e", "102f", "103a",
		"103c", "103f", "1180", "1182", "1183", "1184", "1185", "1186", "1187", "1188", "1189", "118a",
		"118b", "118c", "118d", "118e", "118f", "1191", "1193", "1194", "1195", "1198", "1199", "119a",
		"119d", "119e", "119f", "11a0", "11a1", "11a2", "11a3", "11a7", "11a8", "11a9", "11af", "11b0",
		"11b1", "11b4", "11b6", "11b7", "11b8", "11b9", "11ba", "11bc", "11bd", "11be", "11bf", "11c0",
		"11c2", "11c3", "11c4", "11c5", "11c6", "11c7", "11c8", "11cb", "11e0", "11e1", "11e2", "11e3",
		"11e7", "11fa", "11fc", "1280", "1281", "1282", "1284", "1286", "1287", "1288", "1289", "128a",
		"128b", "128c", "1290", "1291", "1292", "1293", "1294", "1295", "1296", "1298", "1299", "129a",
		"12a0", "12b9", "12ba",
	} {
		keplerIDs[id] = true
	}
}

// IsKepler reports whether a PCI ID (vendor:device) is an NVIDIA Kepler GPU.
func IsKepler(id string) bool {
	vendor, device, _ := strings.Cut(id, ":")
	return vendor == "10de" && keplerIDs[device]
}

// oclpCommit is OCLP's 2.5.1 tag. The kexts are downloaded from that commit, never embedded:
// AMFIPass, RSRHelper and KDKlessWorkaround are published only as binaries inside OCLP.
const oclpCommit = "f40057a5292f4804b51bcfe78d5047c7302a6434"

type oclpKext struct {
	Name, Zip, SHA256, MinKernel string
}

// rootPatchKexts are loaded in this order, after Lilu (RestrictEvents and AMFIPass are Lilu
// plugins). MinKernel follows OCLP's config.plist.
var rootPatchKexts = []oclpKext{
	{"AMFIPass", "Acidanthera/AMFIPass-v1.4.1-RELEASE.zip", "07b266145906db41f4b13a7938fbb173ea28888cc1fa65f84417f8820adc961e", "20.0.0"},
	{"RSRHelper", "Acidanthera/RSRHelper-v1.0.2-RELEASE.zip", "7fa418a7aa35847826a2b7b603987495d5e79644a139866d447e2489b020e623", "20.0.0"},
	{"RestrictEvents", "Acidanthera/RestrictEvents-v1.1.7-RELEASE.zip", "d11e278544a9f1104df60770d2cf2cc2573e8ecaee1dd6d291f4b552975af286", "12.0.0"},
	{"KDKlessWorkaround", "Misc/KDKlessWorkaround-v1.0.0-RELEASE.zip", "8bdb85181d587cf8663e07f2853bc3e6ffa4712300b0dbcf954a97a25da6d0f1", "22.3.0"},
}

// RootPatchKextsDir is the shared cache of those kexts: <Workspace>/.kexts/oclp-2.5.1.
func RootPatchKextsDir() (string, error) {
	dir, err := (Config{Workspace: WorkspaceRoot()}).WorkspaceDir()
	return filepath.Join(dir, ".kexts", "oclp-2.5.1"), err
}

// RootPatchKextsReady reports whether every kext is in the cache.
func RootPatchKextsReady() bool {
	dir, err := RootPatchKextsDir()
	if err != nil {
		return false
	}
	for _, k := range rootPatchKexts {
		if !exists(filepath.Join(dir, k.Name+".kext", "Contents", "MacOS", k.Name)) {
			return false
		}
	}
	return true
}

// oclpBaseURL is where the kexts come from (a variable for tests).
var oclpBaseURL = "https://raw.githubusercontent.com/dortania/OpenCore-Legacy-Patcher/" + oclpCommit + "/payloads/Kexts/"

// DownloadRootPatchKexts fetches the kexts from OCLP's pinned commit, checks each archive's
// SHA-256 and unpacks only its .kext into the cache.
func DownloadRootPatchKexts(ctx context.Context) error {
	dir, err := RootPatchKextsDir()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	for _, k := range rootPatchKexts {
		if exists(filepath.Join(dir, k.Name+".kext", "Contents", "MacOS", k.Name)) {
			continue
		}
		data, err := httpsGet(ctx, oclpBaseURL+k.Zip, 16<<20)
		if err != nil {
			return fmt.Errorf("%s: %w", k.Name, err)
		}
		if sum := sha256.Sum256(data); hex.EncodeToString(sum[:]) != k.SHA256 {
			return fmt.Errorf("%s: the download does not match OCLP 2.5.1's file (SHA-256 %x)", k.Name, sum)
		}
		if err := unpackKext(data, k.Name, dir); err != nil {
			return fmt.Errorf("%s: %w", k.Name, err)
		}
	}
	return nil
}

// unpackKext extracts <name>.kext from a zip into dir, through a temporary directory so a
// partial kext never stays. Entries outside <name>.kext/ (such as __MACOSX) are skipped, and
// none may leave it.
func unpackKext(data []byte, name, dir string) error {
	zr, err := zip.NewReader(bytes.NewReader(data), int64(len(data)))
	if err != nil {
		return err
	}
	tmp, err := os.MkdirTemp(dir, "."+name+"-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(tmp)
	prefix := name + ".kext/"
	for _, f := range zr.File {
		if !strings.HasPrefix(f.Name, prefix) || f.FileInfo().IsDir() {
			continue
		}
		dst := filepath.Join(tmp, filepath.FromSlash(f.Name))
		if !strings.HasPrefix(dst, filepath.Join(tmp, name+".kext")+string(filepath.Separator)) {
			return fmt.Errorf("unexpected path %q in the archive", f.Name)
		}
		if f.Mode()&os.ModeSymlink != 0 {
			return fmt.Errorf("unexpected link %q in the archive", f.Name)
		}
		if err := os.MkdirAll(filepath.Dir(dst), 0o755); err != nil {
			return err
		}
		rc, err := f.Open()
		if err != nil {
			return err
		}
		out, err := os.OpenFile(dst, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o644)
		if err == nil {
			_, err = io.Copy(out, io.LimitReader(rc, 16<<20))
			if cerr := out.Close(); err == nil {
				err = cerr
			}
		}
		rc.Close()
		if err != nil {
			return err
		}
	}
	if !exists(filepath.Join(tmp, name+".kext", "Contents", "MacOS", name)) {
		return errors.New("the archive has no " + name + ".kext")
	}
	final := filepath.Join(dir, name+".kext")
	os.RemoveAll(final)
	return os.Rename(filepath.Join(tmp, name+".kext"), final)
}

// rootPatchMarker, in the VM's boot folder, records that the last boot had root-patch mode on,
// so the first boot with it off deletes the lowered SIP and RestrictEvents values from NVRAM
// (OpenCore's NVRAM Add never replaces a value that is already there).
const rootPatchMarker = BootDir + "/root-patch-on"

// rootPatchPlist is the config.plist text for the mode: on adds the kexts, the lowered SIP and
// revblock; clear only deletes the values a previous "on" boot left in NVRAM.
func rootPatchPlist(on, clear bool) map[string]string {
	r := map[string]string{"@@CSR@@": "AAAAAA==", "@@ROOTPATCHKEXTS@@": "", "@@CSRDELETE@@": "", "@@REVBLOCKADD@@": "", "@@REVBLOCKDELETE@@": ""}
	if on || clear {
		r["@@CSRDELETE@@"] = "\n\t\t\t\t<string>csr-active-config</string>"
		r["@@REVBLOCKDELETE@@"] = "\n\t\t\t\t<string>revblock</string>"
	}
	if !on {
		return r
	}
	r["@@CSR@@"] = "AwgAAA==" // 0x803 little-endian: OCLP's lowered SIP
	r["@@REVBLOCKADD@@"] = "\n\t\t\t\t<key>revblock</key>\n\t\t\t\t<string>media</string>"
	var b strings.Builder
	for _, k := range rootPatchKexts {
		fmt.Fprintf(&b, "\n\t\t\t<dict>\n\t\t\t\t<key>Arch</key>\n\t\t\t\t<string>x86_64</string>\n\t\t\t\t<key>BundlePath</key>\n\t\t\t\t<string>%[1]s.kext</string>"+
			"\n\t\t\t\t<key>Comment</key>\n\t\t\t\t<string>%[1]s.kext (root-patch mode, from OCLP 2.5.1)</string>\n\t\t\t\t<key>Enabled</key>\n\t\t\t\t<true/>"+
			"\n\t\t\t\t<key>ExecutablePath</key>\n\t\t\t\t<string>Contents/MacOS/%[1]s</string>\n\t\t\t\t<key>MaxKernel</key>\n\t\t\t\t<string></string>"+
			"\n\t\t\t\t<key>MinKernel</key>\n\t\t\t\t<string>%[2]s</string>\n\t\t\t\t<key>PlistPath</key>\n\t\t\t\t<string>Contents/Info.plist</string>\n\t\t\t</dict>",
			k.Name, k.MinKernel)
	}
	r["@@ROOTPATCHKEXTS@@"] = b.String()
	return r
}

// rootPatchBootArgs are the boot arguments OCLP adds with lowered SIP.
const rootPatchBootArgs = " ipc_control_port_options=0 -nokcmismatchpanic"

// ClearRootPatchMarker ends the SIP restoration after a VM with the mode off ran long enough
// for OpenCore to have applied its NVRAM Delete (it does so within seconds of power-on).
func (w Workspace) ClearRootPatchMarker(c Config, ran time.Duration) {
	if !c.RootPatch && ran >= time.Minute {
		os.Remove(w.Path(rootPatchMarker))
	}
}

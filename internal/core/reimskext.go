package core

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"path/filepath"
)

// ReimsVgpu.kext (steelbrain/reims-util, GPL-2.0-or-later OR LGPL-3.0-or-later, with Lilu code
// under BSD-3-Clause) lets WindowServer drive Reims vGPU displays wider than 2048 px. It is a
// Lilu plugin (as.vit9696.Lilu 1.7.2, the version the app embeds) that loads on macOS 26 only,
// so Tahoe VMs on Reims vGPU get it (Config.ReimsKext). The boot argument -reimsvgpuoff turns
// it off. It is downloaded from a pinned commit, never embedded.
const reimsUtilCommit = "b302389a957c56fad8e5b3b3b16107932ea9e5e9"

// reimsKextBaseURL is the kext's folder at that commit (a variable for tests).
var reimsKextBaseURL = "https://raw.githubusercontent.com/steelbrain/reims-util/" + reimsUtilCommit + "/kext/ReimsVgpu.kext/"

// reimsKextFiles are the kext's files, relative to ReimsVgpu.kext, with their SHA-256.
var reimsKextFiles = []struct{ Path, SHA256 string }{
	{"Contents/Info.plist", "f572c4a3e5496110f0d6799297a1e215409d4df2e300c22e6070b97d5f3d9b66"},
	{"Contents/MacOS/ReimsVgpu", "d028c738a707af0de69a78b455bbca8ea39e4d8b71a56c51f432bbf1c29915c3"},
}

// reimsKextMinKernel is Darwin 25, macOS 26: the kext does nothing before it.
const reimsKextMinKernel = "25.0.0"

// ReimsKextDir is the cached kext: <Workspace>/.kexts/reims-util-b302389/ReimsVgpu.kext.
func ReimsKextDir() (string, error) {
	dir, err := (Config{Workspace: WorkspaceRoot()}).WorkspaceDir()
	return filepath.Join(dir, ".kexts", "reims-util-"+reimsUtilCommit[:7], "ReimsVgpu.kext"), err
}

// ReimsKextReady reports whether the kext is in the cache.
func ReimsKextReady() bool {
	dir, err := ReimsKextDir()
	if err != nil {
		return false
	}
	for _, f := range reimsKextFiles {
		if !exists(filepath.Join(dir, filepath.FromSlash(f.Path))) {
			return false
		}
	}
	return true
}

// DownloadReimsKext fetches the kext's files from the pinned commit, checks their SHA-256 and
// publishes the kext into the cache only when every file matches.
func DownloadReimsKext(ctx context.Context) error {
	if ReimsKextReady() {
		return nil
	}
	final, err := ReimsKextDir()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(final), 0o755); err != nil {
		return err
	}
	tmp, err := os.MkdirTemp(filepath.Dir(final), ".ReimsVgpu-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(tmp)
	for _, f := range reimsKextFiles {
		data, err := httpsGetMax(ctx, reimsKextBaseURL+f.Path, 4<<20)
		if err != nil {
			return fmt.Errorf("ReimsVgpu.kext: %w", err)
		}
		if sum := sha256.Sum256(data); hex.EncodeToString(sum[:]) != f.SHA256 {
			return fmt.Errorf("ReimsVgpu.kext: %s does not match reims-util %s (SHA-256 %x)", f.Path, reimsUtilCommit[:7], sum)
		}
		dst := filepath.Join(tmp, filepath.FromSlash(f.Path))
		if err := os.MkdirAll(filepath.Dir(dst), 0o755); err != nil {
			return err
		}
		if err := os.WriteFile(dst, data, 0o644); err != nil {
			return err
		}
	}
	if err := os.Rename(tmp, final); err != nil && !ReimsKextReady() { // another process may have won
		return err
	}
	return nil
}

// addReimsKext copies the cached kext into the extracted EFI.
func (w Workspace) addReimsKext() error {
	if !ReimsKextReady() {
		return errors.New("Reims vGPU on Tahoe needs ReimsVgpu.kext: start the VM again to download it")
	}
	src, err := ReimsKextDir()
	if err != nil {
		return err
	}
	if err := copyTree(src, w.Path(EFIDir+"/EFI/OC/Kexts/ReimsVgpu.kext")); err != nil {
		return fmt.Errorf("copying ReimsVgpu.kext: %w", err)
	}
	return nil
}

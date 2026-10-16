package core

import (
	"archive/zip"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"syscall"
	"time"
)

// Extra kexts: OpenCore kexts the user lists in settings.yaml, for example a patch kext for an AMD
// GPU macOS does not drive natively. The app never names, pins or fetches a specific kext: every
// URL comes from the user.

// Archive limits: a size over a limit fails the download, nothing is ever truncated.
const (
	maxKextArchive  = 32 << 20
	maxKextExpanded = 128 << 20
	maxKextFile     = 16 << 20 // unpackKext's per-file limit
	maxKextEntries  = 4096
)

// ExtraKext is one entry of settings.yaml.
type ExtraKext struct {
	URL    string   `yaml:"url"`
	SHA256 string   `yaml:"sha256,omitempty"` // "" = not checked
	Kexts  []string `yaml:"load,omitempty"`   // which kexts to load, in order; nil = the archive's only kext
}

// ExtraKexts is the kexts part of settings.yaml; a start reads it once and uses that snapshot
// for every step.
type ExtraKexts struct {
	Disabled []string    `yaml:"disable,omitempty"` // built-in kexts whose entry is disabled
	Entries  []ExtraKext `yaml:"kexts,omitempty"`
}

var (
	kextSwitchRe = regexp.MustCompile(`@@ENABLED_([A-Za-z0-9]+)@@`)
	kextNameRe   = regexp.MustCompile(`^[A-Za-z0-9._+-]{1,64}$`) // valid on FAT, nothing to escape in XML
	sha256Re     = regexp.MustCompile(`^[0-9a-f]{64}$`)
)

// LoadExtraKexts reads the kexts and disable lists of settings.yaml.
func LoadExtraKexts() (ExtraKexts, error) {
	fs, err := loadSettingsFile()
	if err != nil {
		return ExtraKexts{}, err
	}
	ks := fs.ExtraKexts
	for i := range ks.Entries {
		if err := ks.Entries[i].check(); err != nil {
			return ExtraKexts{}, fmt.Errorf("%s, kext %d (%s): %w", SettingsFile(), i+1, ks.Entries[i].URL, err)
		}
	}
	return ks, nil
}

func (e *ExtraKext) check() error {
	if !strings.HasPrefix(e.URL, "https://") || len(e.URL) == len("https://") || strings.ContainsAny(e.URL, " \t\n") {
		return errors.New("url must be an https:// URL")
	}
	if e.SHA256 = strings.ToLower(e.SHA256); e.SHA256 != "" && !sha256Re.MatchString(e.SHA256) {
		return errors.New("sha256 must be 64 hex digits")
	}
	if len(e.Kexts) == 0 {
		e.Kexts = nil // load: [] is the same as no load list
	}
	for _, n := range e.Kexts {
		if !kextNameRe.MatchString(n) {
			return fmt.Errorf("invalid kext name %q in load", n)
		}
	}
	return nil
}

// id identifies the entry's cache folder: any change to the entry is another folder.
func (e ExtraKext) id() string {
	return e.URL + " sha256=" + e.SHA256 + " load=" + strings.Join(e.Kexts, ":")
}

// ExtraKextsDir is the cache shared by the Workspace's VMs: <Workspace>/.kexts/extra.
func ExtraKextsDir() (string, error) {
	dir, err := (Config{Workspace: WorkspaceRoot()}).WorkspaceDir()
	return filepath.Join(dir, ".kexts", "extra"), err
}

func (e ExtraKext) dir(base string) string {
	sum := sha256.Sum256([]byte(e.id()))
	return filepath.Join(base, hex.EncodeToString(sum[:]))
}

// cached returns the entry's kexts in load order. The folder's "entry" file holds the entry
// text, then one kext name per line; a folder whose text differs is not this entry's.
func (e ExtraKext) cached(base string) ([]string, bool) {
	b, err := os.ReadFile(filepath.Join(e.dir(base), "entry"))
	if err != nil {
		return nil, false
	}
	lines := strings.Split(strings.TrimRight(string(b), "\n"), "\n")
	if lines[0] != e.id() || len(lines) < 2 {
		return nil, false
	}
	for _, n := range lines[1:] {
		for _, f := range []string{filepath.Join("Contents", "MacOS", n), filepath.Join("Contents", "Info.plist")} {
			if st, err := os.Lstat(filepath.Join(e.dir(base), n+".kext", f)); err != nil || !st.Mode().IsRegular() {
				return nil, false
			}
		}
	}
	return lines[1:], true
}

// Ready reports whether every entry is cached, so a start needs no network.
func (ks ExtraKexts) Ready() bool {
	base, err := ExtraKextsDir()
	if err != nil {
		return false
	}
	for _, e := range ks.Entries {
		if _, ok := e.cached(base); !ok {
			return false
		}
	}
	return true
}

// DownloadExtraKexts fetches the entries that are not cached yet. A host-wide lock on the cache
// makes concurrent starts download an entry once; each entry is published by renaming a
// complete temporary folder.
func DownloadExtraKexts(ctx context.Context, ks ExtraKexts) error {
	if len(ks.Entries) == 0 {
		return nil
	}
	base, err := ExtraKextsDir()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(base, 0o755); err != nil {
		return err
	}
	unlock, err := lockFile(ctx, filepath.Join(base, ".lock"))
	if err != nil {
		return err
	}
	defer unlock()
	// Leftovers of a start that died while it held the lock.
	if ents, err := os.ReadDir(base); err == nil {
		for _, e := range ents {
			if strings.HasPrefix(e.Name(), ".tmp-") {
				os.RemoveAll(filepath.Join(base, e.Name()))
			}
		}
	}
	for _, e := range ks.Entries {
		if _, ok := e.cached(base); ok {
			continue
		}
		if err := e.download(ctx, base); err != nil {
			return fmt.Errorf("%s: %w", e.URL, err)
		}
	}
	return ctx.Err()
}

// lockFile takes a host-wide exclusive flock on path, waiting for another holder until ctx
// ends, so a cancelled caller stops waiting.
func lockFile(ctx context.Context, path string) (unlock func(), err error) {
	f, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return nil, err
	}
	for {
		err := syscall.Flock(int(f.Fd()), syscall.LOCK_EX|syscall.LOCK_NB)
		if err == nil {
			return func() { f.Close() }, nil
		}
		if !errors.Is(err, syscall.EWOULDBLOCK) {
			f.Close()
			return nil, err
		}
		select {
		case <-ctx.Done():
			f.Close()
			return nil, ctx.Err()
		case <-time.After(200 * time.Millisecond):
		}
	}
}

func (e ExtraKext) download(ctx context.Context, base string) error {
	data, err := httpsGetMax(ctx, e.URL, maxKextArchive)
	if err != nil {
		return err
	}
	if e.SHA256 != "" {
		if sum := sha256.Sum256(data); hex.EncodeToString(sum[:]) != e.SHA256 {
			return fmt.Errorf("the download's SHA-256 is %x, the entry expects %s", sum, e.SHA256)
		}
	}
	names, err := checkKextArchive(data, e.Kexts)
	if err != nil {
		return err
	}
	tmp, err := os.MkdirTemp(base, ".tmp-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(tmp)
	for _, n := range names {
		if err := unpackKext(data, n, tmp); err != nil {
			return fmt.Errorf("%s.kext: %w", n, err)
		}
	}
	entry := e.id() + "\n" + strings.Join(names, "\n") + "\n"
	if err := os.WriteFile(filepath.Join(tmp, "entry"), []byte(entry), 0o644); err != nil {
		return err
	}
	final := e.dir(base)
	os.RemoveAll(final) // not a valid copy of this entry, or cached() would have taken it
	return os.Rename(tmp, final)
}

// checkKextArchive validates the whole archive before anything is extracted and returns the
// kexts to load, in order: the archive's only root-level kext, or the ones want lists.
func checkKextArchive(data []byte, want []string) ([]string, error) {
	zr, err := zip.NewReader(bytes.NewReader(data), int64(len(data)))
	if err != nil {
		return nil, err
	}
	if len(zr.File) > maxKextEntries {
		return nil, fmt.Errorf("the archive has more than %d entries", maxKextEntries)
	}
	var total uint64
	regular := map[string]bool{}
	var found []string
	for _, f := range zr.File {
		if strings.HasPrefix(f.Name, "/") || slices.Contains(strings.Split(f.Name, "/"), "..") {
			return nil, fmt.Errorf("unexpected path %q in the archive", f.Name)
		}
		if f.Mode()&os.ModeSymlink != 0 {
			return nil, fmt.Errorf("unexpected link %q in the archive", f.Name)
		}
		if f.UncompressedSize64 > maxKextFile {
			return nil, fmt.Errorf("%s is larger than %d MiB", f.Name, maxKextFile>>20)
		}
		if total += f.UncompressedSize64; total > maxKextExpanded {
			return nil, fmt.Errorf("the archive expands to more than %d MiB", maxKextExpanded>>20)
		}
		if !f.Mode().IsRegular() {
			continue
		}
		regular[f.Name] = true
		p := strings.Split(f.Name, "/")
		if len(p) == 4 && p[1] == "Contents" && p[2] == "MacOS" && p[0] == p[3]+".kext" {
			found = append(found, p[3])
		}
	}
	if want == nil {
		switch len(found) {
		case 0:
			return nil, errors.New("the archive has no root-level <Name>.kext/Contents/MacOS/<Name>")
		case 1:
			want = found
		default:
			return nil, fmt.Errorf("the archive has several kexts (%s): choose them, in load order, with load: [%s]", strings.Join(found, ", "), strings.Join(found, ", "))
		}
	}
	for _, n := range want {
		switch {
		case !kextNameRe.MatchString(n):
			return nil, fmt.Errorf("unsupported kext name %q", n)
		case !slices.Contains(found, n):
			return nil, fmt.Errorf("the archive has no %s.kext/Contents/MacOS/%s", n, n)
		case !regular[n+".kext/Contents/Info.plist"]:
			return nil, fmt.Errorf("%s.kext has no Contents/Info.plist", n)
		}
	}
	return want, nil
}

// names returns the snapshot's cached kexts in load order and refuses names that
// clash, ignoring case (OpenCore's disk is FAT), with each other or with taken.
func (ks ExtraKexts) names(base string, taken []string) ([][2]string, error) {
	seen := map[string]string{}
	for _, n := range taken {
		seen[strings.ToLower(n)] = n + ".kext"
	}
	var out [][2]string // {folder, name}
	for _, e := range ks.Entries {
		names, ok := e.cached(base)
		if !ok {
			return nil, fmt.Errorf("the settings.yaml entry %s is not downloaded: start the VM from the app, `vm start` or the MCP server, which download it", e.URL)
		}
		for _, n := range names {
			if other, ok := seen[strings.ToLower(n)]; ok {
				return nil, fmt.Errorf("%s.kext from %s clashes with %s", n, SettingsFile(), other)
			}
			seen[strings.ToLower(n)] = n + ".kext from " + e.URL
			out = append(out, [2]string{e.dir(base), n})
		}
	}
	return out, nil
}

// kextSwitches renders the template's @@ENABLED_<Name>@@ switches: <false/> for the built-in
// kexts settings.yaml disables, <true/> for the others.
func kextSwitches(tmpl string, disabled []string) (string, error) {
	for _, n := range disabled {
		switch {
		case strings.EqualFold(n, "Lilu"):
			return "", errors.New("settings.yaml disable: Lilu is required by every other kext")
		case !strings.Contains(tmpl, "@@ENABLED_"+n+"@@"):
			return "", fmt.Errorf("settings.yaml disable: %s is not a built-in kext the config template in use can disable", n)
		}
	}
	return kextSwitchRe.ReplaceAllStringFunc(tmpl, func(m string) string {
		if slices.Contains(disabled, strings.TrimSuffix(strings.TrimPrefix(m, "@@ENABLED_"), "@@")) {
			return "<false/>"
		}
		return "<true/>"
	}), nil
}

// addExtraKexts copies the snapshot's kexts into the extracted EFI and records them for the
// config, after the built-in and root-patch kexts.
func (w Workspace) addExtraKexts(c *Config, builtin []string) error {
	base, err := ExtraKextsDir()
	if err != nil {
		return err
	}
	taken := slices.Clone(builtin)
	for _, k := range rootPatchKexts {
		taken = append(taken, k.Name)
	}
	kexts, err := c.Extras.names(base, taken)
	if err != nil {
		return err
	}
	for _, k := range kexts {
		if err := copyTree(filepath.Join(k[0], k[1]+".kext"), w.Path(EFIDir+"/EFI/OC/Kexts/"+k[1]+".kext")); err != nil {
			return fmt.Errorf("copying %s.kext: %w", k[1], err)
		}
	}
	c.extraKexts = kexts
	return nil
}

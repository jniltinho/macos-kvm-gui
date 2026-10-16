package core

import (
	"bytes"
	"compress/gzip"
	"context"
	"crypto/sha256"
	"encoding/binary"
	"encoding/json"
	"encoding/xml"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"
)

// The full macOS installer: Apple's InstallAssistant.pkg (Big Sur and later, 12–18 GB), found in
// the Software Update catalog, downloaded once into a cache shared by every VM, checked against
// its integrity data, and served to a running VM over the NAT network so the recovery can install
// from it without downloading macOS again.

// installerCatalogs are the public-release catalogs, newest first (Mist's definitions); the
// Tahoe one lists every InstallAssistant from Big Sur on.
var installerCatalogs = []string{
	"https://swscan.apple.com/content/catalogs/others/index-26-15-14-13-12-10.16-10.15-10.14-10.13-10.12-10.11-10.10-10.9-mountainlion-lion-snowleopard-leopard.merged-1.sucatalog.gz",
	"https://swscan.apple.com/content/catalogs/others/index-26-15-14-13-12-10.16-10.15-10.14-10.13-10.12-10.11-10.10-10.9-mountainlion-lion-snowleopard-leopard.merged-1.sucatalog",
}

// installerMajor is the macOS major version of each app version that has an InstallAssistant.
var installerMajor = map[string]int{"big-sur": 11, "monterey": 12, "ventura": 13, "sonoma": 14, "sequoia": 15, "tahoe": 26}

// boardIDs are the board IDs of the Mac models the VMs present; a product whose distribution
// does not list it is for other Macs (for example Apple silicon only).
var boardIDs = map[string]string{"iMac20,1": "Mac-CFF7D910A743CAAF", "iMac19,1": "Mac-AA95B1DDAB278B95", "iMac18,3": "Mac-BE088AF8C5EB4FA2"}

// InstallerProduct is one InstallAssistant.pkg in the catalog.
type InstallerProduct struct {
	ProductID    string `json:"product_id"`
	Version      string `json:"version"`
	Build        string `json:"build"`
	URL          string `json:"url"`
	Size         int64  `json:"size"`
	IntegrityURL string `json:"integrity_url"`
}

// InstallerSupported reports whether a macOS version has a full installer in the catalog.
func InstallerSupported(version string) bool { _, ok := installerMajor[version]; return ok }

var installerClient = &http.Client{
	Timeout: 60 * time.Second,
	// Apple's catalog, distributions and integrity data must stay on HTTPS.
	CheckRedirect: func(req *http.Request, via []*http.Request) error {
		if req.URL.Scheme != "https" || len(via) > 5 {
			return errors.New("refusing a redirect away from HTTPS")
		}
		return nil
	},
}

// installerDownloadClient has no overall timeout (the package is many GB) but bounded handshakes.
var installerDownloadClient = &http.Client{
	CheckRedirect: installerClient.CheckRedirect,
	Transport: &http.Transport{
		Proxy:                 http.ProxyFromEnvironment,
		TLSHandshakeTimeout:   30 * time.Second,
		ResponseHeaderTimeout: 60 * time.Second,
	},
}

func httpsGet(ctx context.Context, rawURL string, limit int64) ([]byte, error) {
	if !strings.HasPrefix(rawURL, "https://") {
		return nil, fmt.Errorf("refusing a non-HTTPS URL: %s", rawURL)
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, rawURL, nil)
	if err != nil {
		return nil, err
	}
	resp, err := installerClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("%s: %s", rawURL, resp.Status)
	}
	return io.ReadAll(io.LimitReader(resp.Body, limit))
}

// FindInstallers lists the catalog's installers for a macOS version that support the VM's Mac
// model, newest first.
func FindInstallers(ctx context.Context, version string) ([]InstallerProduct, error) {
	major, ok := installerMajor[version]
	if !ok {
		return nil, fmt.Errorf("Apple has no full installer package for %s: use the recovery", versionDisplay(version))
	}
	var cat []byte
	var err error
	for _, u := range installerCatalogs {
		if cat, err = httpsGet(ctx, u, 64<<20); err == nil {
			break
		}
	}
	if err != nil {
		return nil, fmt.Errorf("Apple's software catalog: %w", err)
	}
	if bytes.HasPrefix(cat, []byte{0x1f, 0x8b}) {
		zr, err := gzip.NewReader(bytes.NewReader(cat))
		if err != nil {
			return nil, err
		}
		if cat, err = io.ReadAll(io.LimitReader(zr, 256<<20)); err != nil {
			return nil, err
		}
	}
	candidates, err := catalogInstallers(cat)
	if err != nil {
		return nil, err
	}
	board := boardIDs[SMBIOSModel(version)]
	var (
		mu  sync.Mutex
		out []InstallerProduct
		wg  sync.WaitGroup
		sem = make(chan struct{}, 6)
	)
	for _, c := range candidates {
		wg.Add(1)
		go func(c catalogInstaller) {
			defer wg.Done()
			sem <- struct{}{}
			defer func() { <-sem }()
			dist, err := httpsGet(ctx, c.distURL, 4<<20)
			if err != nil {
				return
			}
			p := c.InstallerProduct
			p.Version, p.Build = distInfo(dist)
			if maj, _ := strconv.Atoi(strings.SplitN(p.Version, ".", 2)[0]); maj != major {
				return
			}
			if board != "" && !bytes.Contains(dist, []byte(board)) {
				return // for other Macs, e.g. Apple silicon only
			}
			mu.Lock()
			out = append(out, p)
			mu.Unlock()
		}(c)
	}
	wg.Wait()
	sort.Slice(out, func(i, j int) bool { return versionLess(out[j].Version, out[i].Version) })
	if len(out) == 0 {
		return nil, fmt.Errorf("Apple's catalog has no full installer of %s for this Mac model", versionDisplay(version))
	}
	return out, nil
}

type catalogInstaller struct {
	InstallerProduct
	distURL string
}

// catalogInstallers finds the products that carry an InstallAssistant.pkg.
func catalogInstallers(catalog []byte) ([]catalogInstaller, error) {
	root, err := parsePlist(catalog)
	if err != nil {
		return nil, fmt.Errorf("Apple's software catalog: %w", err)
	}
	products, _ := plistDict(root)["Products"].(map[string]any)
	var out []catalogInstaller
	for id, v := range products {
		p, _ := v.(map[string]any)
		meta, _ := p["ExtendedMetaInfo"].(map[string]any)
		if _, ok := meta["InstallAssistantPackageIdentifiers"]; !ok {
			continue
		}
		dists, _ := p["Distributions"].(map[string]any)
		dist, _ := dists["English"].(string)
		if dist == "" {
			dist, _ = dists["en"].(string)
		}
		pkgs, _ := p["Packages"].([]any)
		for _, pv := range pkgs {
			pk, _ := pv.(map[string]any)
			u, _ := pk["URL"].(string)
			if !strings.HasSuffix(u, "/InstallAssistant.pkg") || dist == "" {
				continue
			}
			size, _ := pk["Size"].(int64)
			integrity, _ := pk["IntegrityDataURL"].(string)
			out = append(out, catalogInstaller{InstallerProduct{ProductID: id, URL: u, Size: size, IntegrityURL: integrity}, dist})
		}
	}
	return out, nil
}

var (
	distVersionRe = regexp.MustCompile(`<key>(?:VERSION|macOSProductVersion)</key>\s*<string>([^<]+)</string>`)
	distBuildRe   = regexp.MustCompile(`<key>(?:BUILD|macOSProductBuildVersion)</key>\s*<string>([^<]+)</string>`)
)

func distInfo(dist []byte) (version, build string) {
	if m := distVersionRe.FindSubmatch(dist); m != nil {
		version = string(m[1])
	}
	if m := distBuildRe.FindSubmatch(dist); m != nil {
		build = string(m[1])
	}
	return version, build
}

// versionLess compares dotted versions numerically (15.10 > 15.9).
func versionLess(a, b string) bool {
	pa, pb := strings.Split(a, "."), strings.Split(b, ".")
	for i := 0; i < max(len(pa), len(pb)); i++ {
		var x, y int
		if i < len(pa) {
			x, _ = strconv.Atoi(pa[i])
		}
		if i < len(pb) {
			y, _ = strconv.Atoi(pb[i])
		}
		if x != y {
			return x < y
		}
	}
	return false
}

// --- integrity data -------------------------------------------------------------------------

// parseIntegrity reads InstallAssistant's integrity data: a CNKL chunklist with signature
// method 2, whose last 32 bytes are the SHA-256 of everything before them. It is not signed by
// Apple (unlike the recovery's chunklist); its authenticity comes from Apple's HTTPS servers.
func parseIntegrity(data []byte) ([]chunk, error) {
	r := bytes.NewReader(data)
	var h chunklistHeader
	if err := binary.Read(r, binary.LittleEndian, &h); err != nil {
		return nil, fmt.Errorf("integrity data truncated: %w", err)
	}
	if string(h.Magic[:]) != "CNKL" || h.HeaderSize != headerSize || h.ChunkMethod != 1 || h.ChunkCount == 0 ||
		h.ChunkCount > uint64(len(data))/chunkSize || h.ChunkOffset != headerSize || h.SignatureOffset != headerSize+chunkSize*h.ChunkCount {
		return nil, errors.New("integrity data has an invalid header")
	}
	if h.SignatureMethod != 2 || uint64(len(data)) != h.SignatureOffset+32 {
		return nil, errors.New("integrity data has an unexpected checksum format")
	}
	sum := sha256.Sum256(data[:h.SignatureOffset])
	if !bytes.Equal(sum[:], data[h.SignatureOffset:]) {
		return nil, errors.New("integrity data is corrupted")
	}
	chunks := make([]chunk, h.ChunkCount)
	if err := binary.Read(r, binary.LittleEndian, chunks); err != nil {
		return nil, err
	}
	return chunks, nil
}

// verifyChunks checks a file against its chunks, reporting progress in bytes.
func verifyChunks(ctx context.Context, path string, chunks []chunk, progress func(done, total int64)) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()
	var total, done int64
	for _, c := range chunks {
		total += int64(c.Size)
	}
	if st, err := f.Stat(); err != nil || st.Size() != total {
		return errors.New("package integrity check failed: wrong size")
	}
	buf := make([]byte, 0, 16<<20)
	for i, c := range chunks {
		if err := ctx.Err(); err != nil {
			return err
		}
		buf = buf[:c.Size]
		if _, err := io.ReadFull(f, buf); err != nil {
			return fmt.Errorf("package integrity check failed: truncated at chunk %d", i+1)
		}
		if sha256.Sum256(buf) != c.Hash {
			return fmt.Errorf("package integrity check failed: chunk %d does not match", i+1)
		}
		done += int64(c.Size)
		if progress != nil && (i%64 == 0 || i == len(chunks)-1) {
			progress(done, total)
		}
	}
	return nil
}

// --- cache -----------------------------------------------------------------------------------

const installerFile = "InstallAssistant.pkg" // only while downloading: the cache keeps the disc

// CachedInstaller is a verified installer in the shared cache, as an installer disc.
type CachedInstaller struct {
	InstallerProduct
	Path string `json:"-"` // the .iso
}

// InstallersDir is the shared cache: <Workspace>/.installers/<build>/.
func InstallersDir() (string, error) {
	dir, err := (Config{Workspace: WorkspaceRoot()}).WorkspaceDir()
	return filepath.Join(dir, ".installers"), err
}

// CachedInstallers lists the verified installers, newest first.
func CachedInstallers() []CachedInstaller {
	dir, err := InstallersDir()
	if err != nil {
		return nil
	}
	entries, _ := os.ReadDir(dir)
	var out []CachedInstaller
	for _, e := range entries {
		b, err := os.ReadFile(filepath.Join(dir, e.Name(), "info.json"))
		if err != nil {
			continue
		}
		var c CachedInstaller
		if json.Unmarshal(b, &c) != nil || !bridgeNameRe.MatchString(c.Build) {
			continue
		}
		c.Path = filepath.Join(dir, e.Name(), discFile)
		if st, err := os.Stat(c.Path); err == nil && st.Size() > c.Size {
			out = append(out, c)
		}
	}
	sort.Slice(out, func(i, j int) bool { return versionLess(out[j].Version, out[i].Version) })
	return out
}

// CachedInstallerFor is the newest cached installer of a macOS version, if any.
func CachedInstallerFor(version string) (CachedInstaller, bool) {
	major := installerMajor[version]
	for _, c := range CachedInstallers() {
		if maj, _ := strconv.Atoi(strings.SplitN(c.Version, ".", 2)[0]); major != 0 && maj == major {
			return c, true
		}
	}
	return CachedInstaller{}, false
}

// RemoveInstaller deletes a cached installer; one attached to a running VM is kept.
func RemoveInstaller(build string) error {
	dir, err := InstallersDir()
	if err != nil {
		return err
	}
	if !bridgeNameRe.MatchString(build) {
		return fmt.Errorf("invalid build %q", build)
	}
	path := filepath.Join(dir, build)
	if discInUse(filepath.Join(path, discFile)) {
		return errors.New("a running VM is using this installer: shut it down first")
	}
	return os.RemoveAll(path)
}

// DownloadInstaller downloads (resuming a partial download), verifies and caches the newest full
// installer of the macOS version, unless it is already cached.
func DownloadInstaller(ctx context.Context, version string, progress func(Progress), log func(string)) (CachedInstaller, error) {
	report := func(stage string, pct float64) {
		if progress != nil {
			progress(Progress{Stage: stage, Pct: pct})
		}
	}
	report("Reading Apple's software catalog", -1)
	products, err := FindInstallers(ctx, version)
	if err != nil {
		return CachedInstaller{}, err
	}
	p := products[0]
	if c, ok := CachedInstallerFor(version); ok && c.Build == p.Build {
		return c, nil
	}
	if log != nil {
		log(fmt.Sprintf("[installer] macOS %s (%s), %.1f GB from %s", p.Version, p.Build, float64(p.Size)/1e9, p.URL))
	}
	root, err := InstallersDir()
	if err != nil {
		return CachedInstaller{}, err
	}
	dir := filepath.Join(root, p.Build)
	if !bridgeNameRe.MatchString(p.Build) {
		return CachedInstaller{}, fmt.Errorf("unexpected build %q in Apple's catalog", p.Build)
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return CachedInstaller{}, err
	}
	// One download per build, across processes (the app, the CLI, the MCP server): they share
	// the .part file and the disc being written.
	lock, err := os.OpenFile(filepath.Join(dir, ".lock"), os.O_CREATE|os.O_RDWR, 0o644)
	if err != nil {
		return CachedInstaller{}, err
	}
	defer lock.Close()
	if err := syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		return CachedInstaller{}, errors.New("this installer is already being downloaded (by the app, the command line or an agent)")
	}
	if p.IntegrityURL == "" {
		return CachedInstaller{}, errors.New("Apple's catalog has no integrity data for this installer")
	}
	integrity, err := httpsGet(ctx, p.IntegrityURL, 8<<20)
	if err != nil {
		return CachedInstaller{}, fmt.Errorf("integrity data: %w", err)
	}
	chunks, err := parseIntegrity(integrity)
	if err != nil {
		return CachedInstaller{}, err
	}
	part := filepath.Join(dir, installerFile+".part")
	if exists(filepath.Join(dir, installerFile)) { // verified, but its disc was not written
		os.Rename(filepath.Join(dir, installerFile), part)
	}
	have := int64(0)
	if st, err := os.Stat(part); err == nil {
		have = st.Size()
	}
	// The package, then the disc written from it: twice its size for a moment.
	if free, err := freeBytes(dir); err == nil && free < 2*p.Size-have+(1<<30) {
		return CachedInstaller{}, fmt.Errorf("not enough free space in %s: %.1f GB needed, %.1f GB free", root, float64(2*p.Size-have)/1e9, float64(free)/1e9)
	}
	if err := resumeDownload(ctx, p.URL, part, p.Size, func(done, total int64) {
		report(fmt.Sprintf("Downloading the macOS %s installer (%.1f GB)", p.Version, float64(total)/1e9), float64(done)*100/float64(total))
	}); err != nil {
		return CachedInstaller{}, err
	}
	if err := verifyChunks(ctx, part, chunks, func(done, total int64) {
		report("Checking the package against Apple's integrity data", float64(done)*100/float64(total))
	}); err != nil {
		if ctx.Err() == nil {
			os.Remove(part) // a bad download is not resumed; a cancelled check keeps it for next time
		}
		return CachedInstaller{}, err
	}
	report("Writing the installer disc", -1)
	final := filepath.Join(dir, discFile)
	if err := writeInstallerDisc(ctx, part, final); err != nil { // cancelled: the package stays for next time
		return CachedInstaller{}, err
	}
	os.Remove(part)
	info, _ := json.MarshalIndent(p, "", "  ")
	if err := writeFileAtomic(filepath.Join(dir, "info.json"), info, 0o644); err != nil {
		return CachedInstaller{}, err
	}
	return CachedInstaller{InstallerProduct: p, Path: final}, nil
}

// resumeDownload appends to part from where it stopped, with an HTTP Range request. A server
// that answers 200 instead of 206 restarts the file; a 416 means it is already complete.
func resumeDownload(ctx context.Context, rawURL, part string, size int64, progress func(done, total int64)) error {
	if !strings.HasPrefix(rawURL, "https://") {
		rawURL = "https://" + strings.TrimPrefix(rawURL, "http://") // Apple's CDN serves HTTPS too
	}
	for attempt := 0; ; attempt++ {
		have := int64(0)
		if st, err := os.Stat(part); err == nil {
			have = st.Size()
		}
		if have == size {
			return nil
		}
		if have > size {
			os.Remove(part)
			have = 0
		}
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, rawURL, nil)
		if err != nil {
			return err
		}
		if have > 0 {
			req.Header.Set("Range", fmt.Sprintf("bytes=%d-", have))
		}
		resp, err := installerDownloadClient.Do(req)
		if err != nil {
			if ctx.Err() != nil || attempt >= 5 {
				return fmt.Errorf("download interrupted (it resumes next time): %w", err)
			}
			time.Sleep(time.Duration(attempt+1) * 2 * time.Second)
			continue
		}
		flags := os.O_WRONLY | os.O_CREATE | os.O_APPEND
		switch {
		case resp.StatusCode == http.StatusPartialContent && have > 0 &&
			strings.HasPrefix(resp.Header.Get("Content-Range"), fmt.Sprintf("bytes %d-", have)):
		case resp.StatusCode == http.StatusOK:
			flags, have = os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0 // the server ignored the range: start over
		case resp.StatusCode == http.StatusRequestedRangeNotSatisfiable:
			resp.Body.Close()
			os.Remove(part)
			continue
		default:
			resp.Body.Close()
			return fmt.Errorf("download: %s", resp.Status)
		}
		f, err := os.OpenFile(part, flags, 0o644)
		if err != nil {
			resp.Body.Close()
			return err
		}
		done := have
		buf := make([]byte, 1<<20)
		last := time.Time{}
		var werr error
		for {
			n, rerr := resp.Body.Read(buf)
			if n > 0 {
				if _, werr = f.Write(buf[:n]); werr != nil {
					break
				}
				done += int64(n)
				if progress != nil && time.Since(last) > 250*time.Millisecond {
					progress(done, size)
					last = time.Now()
				}
			}
			if rerr != nil {
				if rerr != io.EOF {
					werr = rerr
				}
				break
			}
		}
		resp.Body.Close()
		if cerr := f.Close(); werr == nil {
			werr = cerr
		}
		if werr != nil {
			if ctx.Err() != nil || attempt >= 5 {
				return fmt.Errorf("download interrupted (it resumes next time): %w", werr)
			}
			continue
		}
		if progress != nil {
			progress(done, size)
		}
	}
}

// --- installer disc -------------------------------------------------------------------------

// The cached installer is an ISO 9660 disc that the VM gets as a disk while it has install media,
// so the recovery installs with no network. macOS's cd9660 reads no multi-extent files (one over
// 4 GiB shows as several 4 GiB ones), so the package is stored as consecutive pieces, PKG.000,
// PKG.001, … that the script joins.

const (
	discFile   = "InstallAssistant.iso"
	discVolume = "OFFLINE"
	// DiscCommand is what to type in the recovery's Terminal; DiscTyped is the same for the app
	// to type (see GuestKeys), and " -u" makes either unattended.
	DiscCommand = "sh /Volumes/" + discVolume + "/INSTALL.SH"
	DiscTyped   = "sh /V*/" + discVolume + "/I*"
)

var discPiece int64 = 1<<32 - 2048 // the largest sector-aligned size one extent holds (a var for tests)

// discScript copies the recovery's own installer app onto the erased disk (the recovery's
// volume is read-only), joins the package into it as its SharedSupport.dmg and opens it. With
// -u it first erases the VM's empty disk, and installs with startosinstall, accepting Apple's
// license, with no questions.
const discScript = `#!/bin/sh
# macos-kvm-gui: install macOS from the full installer disc, with no network.
# sh INSTALL.SH [-u] [/Volumes/Target]   -u: unattended (erases the empty disk, accepts the license)
set -e
export LC_ALL=C
U=
if [ "$1" = -u ]; then U=1; shift; fi
T="${1:-/Volumes/Macintosh HD}"
T="${T%/}"
# Big Sur's recovery has no dirname or basename: the shell's own expansions instead.
case "$0" in */*) D="${0%/*}" ;; *) D=. ;; esac
N="${T##*/}"
if [ -z "$N" ]; then echo "Give a volume such as \"/Volumes/Macintosh HD\""; exit 1; fi
if [ -n "$U" ] && [ ! -d "$T" ]; then
  # The VM's new disk: the only internal disk whose first MiB reads back whole and all zeros.
  # A disk with any partition map, file system or the installer disc has data there; a disk
  # that cannot be read is skipped, never erased.
  E=
  for d in $(diskutil list internal physical | awk '/^\/dev\/disk[0-9]+/ {print $1}'); do
    # One read into a sample: it must succeed and give the whole MiB (n), with no byte but
    # zero (z). The recovery has no cmp; tr reads bytes only with LC_ALL=C.
    S=/tmp/mkg-sample
    rm -f "$S"
    if dd if="$d" of="$S" bs=1048576 count=1 2>/dev/null; then
      n=$(wc -c < "$S" | tr -d ' ')
      z=$(tr -d '\000' < "$S" | wc -c | tr -d ' ')
      if [ "$n" = 1048576 ] && [ "$z" = 0 ]; then E="$E $d"; fi
    fi
    rm -f "$S"
  done
  set -- $E
  if [ $# -ne 1 ]; then
    echo "Found $# empty disks, not one: erase the disk in Disk Utility as APFS named \"$N\" and run this again"
    exit 1
  fi
  echo "== Erasing $1 as APFS \"$N\""
  diskutil eraseDisk APFS "$N" GPT "$1"
fi
if [ ! -d "$T" ]; then
  echo "No volume at $T. Erase the disk in Disk Utility as APFS named \"Macintosh HD\", or run: sh $0 \"/Volumes/Your Disk\""
  exit 1
fi
SRC=
for a in /Install\ macOS*.app; do if [ -d "$a" ]; then SRC="$a"; break; fi; done
if [ -z "$SRC" ]; then echo "This recovery has no installer app: boot the VM's macOS recovery"; exit 1; fi
APP="$T/private/tmp/${SRC##*/}"
echo "== Copying ${SRC##*/} to $T"
mkdir -p "$T/private/tmp"
rm -rf "$APP"
cp -R "$SRC" "$APP"
mkdir -p "$APP/Contents/SharedSupport"
echo "== Copying the installer package from the disc (a few minutes)"
cat "$D"/[Pp][Kk][Gg].[0-9][0-9][0-9] > "$APP/Contents/SharedSupport/SharedSupport.dmg"
if [ -n "$U" ]; then
  echo "== Installing on $T; the VM restarts by itself, and macOS is ready at Setup Assistant"
  if "$APP/Contents/Resources/startosinstall" --agreetolicense --volume "$T"; then
    # In the recovery its restart request goes nowhere: restart once it is prepared.
    sleep 10
    reboot
    exit 0
  fi
  echo "startosinstall failed: opening the installer instead"
fi
echo "== Opening the installer: continue there and choose $N"
for x in InstallAssistant_springboard InstallAssistant; do
  if [ -x "$APP/Contents/MacOS/$x" ]; then exec "$APP/Contents/MacOS/$x"; fi
done
echo "Could not find the installer in $APP"
exit 1
`

// useDisc holds a shared flock on a disc until the returned func is called; discInUse tries
// an exclusive one, so a running VM's disc is seen from any process (the app or the CLI).
func useDisc(p string) (func(), error) {
	f, err := os.Open(p)
	if err != nil {
		return nil, err
	}
	if err := syscall.Flock(int(f.Fd()), syscall.LOCK_SH); err != nil {
		f.Close()
		return nil, err
	}
	return func() { f.Close() }, nil
}

func discInUse(p string) bool {
	f, err := os.Open(p)
	if err != nil {
		return false
	}
	defer f.Close()
	return syscall.Flock(int(f.Fd()), syscall.LOCK_EX|syscall.LOCK_NB) != nil
}

const discScriptSectors = 16 // 32 KiB kept for the script, so a newer one fits in place

// discHead is everything before the package on a disc for a package of pkgSize bytes: the
// volume descriptors (sectors 16, 17), path tables (18, 19), the root directory (20) and the
// script (21–36). The pieces follow back to back from pkgLBA, so the package is one contiguous
// copy. It depends only on pkgSize, so a cached disc's head can be rewritten with a newer script.
func discHead(pkgSize int64) (head []byte, pkgLBA, total uint32, err error) {
	type file struct {
		name      string
		lba, size uint32
	}
	const scriptLBA = 21
	if len(discScript) > discScriptSectors*2048 {
		return nil, 0, 0, errors.New("installer disc: script too large")
	}
	files := []file{{"INSTALL.SH;1", scriptLBA, uint32(len(discScript))}}
	pkgLBA = scriptLBA + discScriptSectors
	next := pkgLBA
	for off, n := int64(0), 0; off < pkgSize; off, n = off+discPiece, n+1 {
		size := min(pkgSize-off, discPiece)
		files = append(files, file{fmt.Sprintf("PKG.%03d;1", n), next, uint32(size)})
		next += uint32((size + 2047) / 2048)
	}
	total = next

	both32 := func(b []byte, v uint32) { binary.LittleEndian.PutUint32(b, v); binary.BigEndian.PutUint32(b[4:], v) }
	both16 := func(b []byte, v uint16) { binary.LittleEndian.PutUint16(b, v); binary.BigEndian.PutUint16(b[2:], v) }
	record := func(name string, lba, size uint32, dir bool) []byte {
		r := make([]byte, 33+len(name)+(1-len(name)%2))
		r[0] = byte(len(r))
		both32(r[2:], lba)
		both32(r[10:], size)
		r[18], r[19], r[20] = 120, 1, 1 // 2020-01-01: a fixed date keeps the head reproducible
		if dir {
			r[25] = 2
		}
		both16(r[28:], 1)
		r[32] = byte(len(name))
		copy(r[33:], name)
		return r
	}
	root := append(record("\x00", 20, 2048, true), record("\x01", 20, 2048, true)...)
	for _, f := range files {
		root = append(root, record(f.name, f.lba, f.size, false)...)
	}
	if len(root) > 2048 {
		return nil, 0, 0, errors.New("installer disc: too many pieces")
	}

	head = make([]byte, int(pkgLBA)*2048)
	pvd := head[16*2048:]
	pad := func(b []byte) {
		for i := range b {
			b[i] = ' '
		}
	}
	pvd[0], pvd[6] = 1, 1
	copy(pvd[1:], "CD001")
	pad(pvd[8:72])
	copy(pvd[40:], discVolume)
	both32(pvd[80:], total)
	both16(pvd[120:], 1)
	both16(pvd[124:], 1)
	both16(pvd[128:], 2048)
	both32(pvd[132:], 10) // one path table entry: the root
	binary.LittleEndian.PutUint32(pvd[140:], 18)
	binary.BigEndian.PutUint32(pvd[148:], 19)
	copy(pvd[156:], record("\x00", 20, 2048, true))
	pad(pvd[190:813])
	for _, at := range []int{813, 830, 847, 864} { // creation, modification, expiration, effective: unset
		for i := range 16 {
			pvd[at+i] = '0'
		}
	}
	pvd[881] = 1
	term := head[17*2048:]
	term[0], term[6] = 255, 1
	copy(term[1:], "CD001")
	lpt, mpt := head[18*2048:], head[19*2048:]
	lpt[0], mpt[0] = 1, 1
	binary.LittleEndian.PutUint32(lpt[2:], 20)
	binary.LittleEndian.PutUint16(lpt[6:], 1)
	binary.BigEndian.PutUint32(mpt[2:], 20)
	binary.BigEndian.PutUint16(mpt[6:], 1)
	copy(head[20*2048:], root)
	copy(head[scriptLBA*2048:], discScript)
	return head, pkgLBA, total, nil
}

// writeInstallerDisc writes the disc iso from the package pkg; cancelling ctx stops it and
// leaves no disc.
func writeInstallerDisc(ctx context.Context, pkg, iso string) error {
	src, err := os.Open(pkg)
	if err != nil {
		return err
	}
	defer src.Close()
	st, err := src.Stat()
	if err != nil {
		return err
	}
	head, pkgLBA, total, err := discHead(st.Size())
	if err != nil {
		return err
	}
	tmp := iso + ".tmp"
	out, err := os.Create(tmp)
	if err != nil {
		return err
	}
	defer os.Remove(tmp)
	_, err = out.Write(head)
	if err == nil {
		_, err = out.Seek(int64(pkgLBA)*2048, io.SeekStart)
	}
	for err == nil { // 1 GiB at a time: copy_file_range stays in the kernel, and a cancel is seen
		if err = ctx.Err(); err != nil {
			break
		}
		var n int64
		if n, err = io.CopyN(out, src, 1<<30); err == io.EOF || (err == nil && n < 1<<30) {
			err = nil
			break
		}
	}
	if err == nil {
		err = out.Truncate(int64(total) * 2048)
	}
	if cerr := out.Close(); err == nil {
		err = cerr
	}
	if err != nil {
		return fmt.Errorf("installer disc: %w", err)
	}
	return os.Rename(tmp, iso)
}

// RefreshInstallerDisc rewrites a cached disc's head when this version's script differs, so
// discs written by an older version get the current one.
func RefreshInstallerDisc(c CachedInstaller) error {
	head, _, total, err := discHead(c.Size)
	if err != nil {
		return err
	}
	f, err := os.OpenFile(c.Path, os.O_RDWR, 0)
	if err != nil {
		return err
	}
	defer f.Close()
	if st, err := f.Stat(); err != nil || st.Size() != int64(total)*2048 {
		return errors.New("installer disc: unexpected size; delete the installer and download it again")
	}
	old := make([]byte, len(head))
	if _, err := io.ReadFull(f, old); err != nil {
		return err
	}
	if bytes.Equal(old, head) {
		return nil
	}
	if _, err := f.WriteAt(head, 0); err != nil {
		return err
	}
	return f.Close()
}

func installerName(version string) string {
	major, _ := strconv.Atoi(strings.SplitN(version, ".", 2)[0])
	for short, m := range installerMajor {
		if m == major {
			name, _, _ := strings.Cut(versionDisplay(short), " (")
			return name
		}
	}
	return version
}

// --- a small XML property-list reader (no dependency) ----------------------------------------

// parsePlist decodes an XML plist into map[string]any, []any, string, int64, bool and []byte.
func parsePlist(data []byte) (any, error) {
	d := xml.NewDecoder(bytes.NewReader(data))
	d.Strict = false
	for {
		tok, err := d.Token()
		if err != nil {
			return nil, err
		}
		if se, ok := tok.(xml.StartElement); ok && se.Name.Local != "plist" {
			return plistValue(d, se)
		}
	}
}

func plistValue(d *xml.Decoder, se xml.StartElement) (any, error) {
	switch se.Name.Local {
	case "dict":
		m := map[string]any{}
		key := ""
		for {
			tok, err := d.Token()
			if err != nil {
				return nil, err
			}
			switch t := tok.(type) {
			case xml.StartElement:
				if t.Name.Local == "key" {
					var s string
					if err := d.DecodeElement(&s, &t); err != nil {
						return nil, err
					}
					key = s
					continue
				}
				v, err := plistValue(d, t)
				if err != nil {
					return nil, err
				}
				m[key] = v
			case xml.EndElement:
				return m, nil
			}
		}
	case "array":
		var a []any
		for {
			tok, err := d.Token()
			if err != nil {
				return nil, err
			}
			switch t := tok.(type) {
			case xml.StartElement:
				v, err := plistValue(d, t)
				if err != nil {
					return nil, err
				}
				a = append(a, v)
			case xml.EndElement:
				return a, nil
			}
		}
	case "true", "false":
		if err := d.Skip(); err != nil {
			return nil, err
		}
		return se.Name.Local == "true", nil
	}
	var s string
	if err := d.DecodeElement(&s, &se); err != nil {
		return nil, err
	}
	if se.Name.Local == "integer" {
		n, err := strconv.ParseInt(strings.TrimSpace(s), 10, 64)
		return n, err
	}
	return s, nil
}

func plistDict(v any) map[string]any { m, _ := v.(map[string]any); return m }

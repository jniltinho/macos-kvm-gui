package core

import (
	"archive/tar"
	"bufio"
	"bytes"
	"compress/gzip"
	"context"
	"crypto/sha256"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"time"
)

// Installing Reims vGPU's QEMU: a prebuilt package from this repository's Releases (pinned by the
// embedded manifest), or a Docker build of upstream at the commit the embedded Dockerfile pins.
// Both land in <ReimsDir>/qemu/<build>, and qemu/current is switched to a build only once it
// works on this host, so a broken build never replaces a working one.

var (
	//go:embed reims/Dockerfile
	reimsDockerfile []byte
	//go:embed reims/manifest.json
	reimsManifestJSON []byte
)

// ReimsReleases are the Ubuntu releases packages and Docker builds exist for.
var ReimsReleases = []string{"24.04", "26.04"}

// ReimsOtherPlatforms are other hosts with a prebuilt package (installed, not built here):
// Debian 13 and Rocky Linux 10 (the RHEL 10 family). These keys name their packages and folders.
var ReimsOtherPlatforms = []string{"debian13", "rocky10"}

// reimsSuffix is a platform key's build-folder suffix: -ubuntu24.04, -debian13, -rocky10.
func reimsSuffix(key string) string {
	if slices.Contains(ReimsReleases, key) {
		return "-ubuntu" + key
	}
	return "-" + key
}

// reimsReleaseURL is where the manifest's tag is published (a var for tests).
var reimsReleaseURL = "https://github.com/jniltinho/macos-kvm-gui/releases/download/"

var (
	osReleaseFile = "/etc/os-release" // vars for tests
	lddCommand    = "ldd"
)

// Package limits: a size over a limit fails the install, nothing is ever truncated.
const (
	maxReimsPackage = 512 << 20
	maxReimsEntries = 20000
)

var maxReimsExpanded int64 = 2 << 30 // a var for tests

type reimsPackage struct {
	File   string `json:"file"`
	SHA256 string `json:"sha256"`
}

type reimsManifest struct {
	Tag      string                  `json:"tag"`
	Commit   string                  `json:"commit"`
	Packages map[string]reimsPackage `json:"packages"`
}

func loadReimsManifest() (m reimsManifest, err error) {
	err = json.Unmarshal(reimsManifestJSON, &m)
	return m, err
}

// HostPlatform returns the host's platform key: its Ubuntu release when one of ReimsReleases,
// else "debian13" for Debian 13 or "rocky10" for Rocky Linux, RHEL or AlmaLinux 10.
func HostPlatform() (string, error) {
	data, err := os.ReadFile(osReleaseFile)
	if err != nil {
		return "", err
	}
	kv := map[string]string{}
	for _, line := range strings.Split(string(data), "\n") {
		if k, v, ok := strings.Cut(strings.TrimSpace(line), "="); ok {
			kv[k] = strings.Trim(v, `"'`)
		}
	}
	id, version := kv["ID"], kv["VERSION_ID"]
	rhel := slices.Contains([]string{"rocky", "rhel", "almalinux"}, id) || slices.Contains(strings.Fields(kv["ID_LIKE"]), "rhel")
	switch {
	case id == "ubuntu" && slices.Contains(ReimsReleases, version):
		return version, nil
	case id == "debian" && version == "13":
		return "debian13", nil
	case rhel && strings.SplitN(version, ".", 2)[0] == "10":
		return "rocky10", nil
	}
	name := kv["PRETTY_NAME"]
	if name == "" {
		name = "not identified by " + osReleaseFile
	}
	return "", fmt.Errorf("Reims vGPU supports Ubuntu %s, Debian 13 and Rocky Linux 10 (RHEL 10 family) hosts, and this one is %s", strings.Join(ReimsReleases, " and "), name)
}

// InstallReims downloads the package for the host's release, checks it against the manifest,
// unpacks it and activates it.
func InstallReims(ctx context.Context, log func(string)) error {
	rel, err := HostPlatform()
	if err != nil {
		return fmt.Errorf("%w: there is no package for it (`macos-kvm-gui reims build --release 24.04|26.04` builds an Ubuntu one in Docker)", err)
	}
	m, err := loadReimsManifest()
	if err != nil {
		return err
	}
	pkg, ok := m.Packages[rel]
	if !ok || !sha256Re.MatchString(pkg.SHA256) {
		return fmt.Errorf("the app has no Reims vGPU package for %s: build it with `macos-kvm-gui reims build` (Ubuntu)", rel)
	}
	qdir, unlock, err := lockReims(ctx)
	if err != nil {
		return err
	}
	defer unlock()
	name := m.Tag + reimsSuffix(rel)
	if _, err := os.Stat(filepath.Join(qdir, name)); err == nil {
		log(name + " is already unpacked")
	} else if err := fetchReimsPackage(ctx, reimsReleaseURL+m.Tag+"/"+pkg.File, pkg.SHA256, filepath.Join(qdir, name), log); err != nil {
		return err
	}
	return activateReims(qdir, name, rel, log)
}

// lockReims takes the host-wide lock held through a whole install or build, including the
// switch of current, and returns <ReimsDir>/qemu.
func lockReims(ctx context.Context) (string, func(), error) {
	dir, err := ReimsDir()
	if err != nil {
		return "", nil, err
	}
	if dir, err = filepath.Abs(dir); err != nil {
		return "", nil, err
	}
	qdir := filepath.Join(dir, "qemu")
	if err := os.MkdirAll(qdir, 0o755); err != nil {
		return "", nil, err
	}
	unlock, err := lockFile(ctx, filepath.Join(dir, ".lock"))
	if err != nil {
		return "", nil, err
	}
	// Leftovers of an install that died while it held the lock.
	tmps, _ := filepath.Glob(filepath.Join(qdir, ".tmp-*"))
	for _, t := range tmps {
		os.RemoveAll(t)
	}
	return qdir, unlock, nil
}

// fetchReimsPackage downloads url, checks its SHA-256 and unpacks it into final, which only
// appears once complete.
func fetchReimsPackage(ctx context.Context, url, want, final string, log func(string)) error {
	tmp, err := os.MkdirTemp(filepath.Dir(final), ".tmp-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(tmp)
	file := filepath.Join(tmp, "package")
	log("Downloading " + url)
	if err := downloadTo(ctx, url, file, maxReimsPackage); err != nil {
		return err
	}
	f, err := os.Open(file)
	if err != nil {
		return err
	}
	defer f.Close()
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return err
	}
	if got := hex.EncodeToString(h.Sum(nil)); got != want {
		return fmt.Errorf("%s: the download's SHA-256 is %s, the app expects %s", url, got, want)
	}
	f.Seek(0, io.SeekStart)
	log("Unpacking into " + final)
	build := filepath.Join(tmp, "build")
	if err := unpackReims(f, build); err != nil {
		return fmt.Errorf("%s: %w", url, err)
	}
	return os.Rename(build, final)
}

// downloadTo saves an HTTPS URL into file, failing past max bytes. The download client has no
// overall timeout, as a package takes a while on a slow line.
func downloadTo(ctx context.Context, url, file string, max int64) error {
	if !strings.HasPrefix(url, "https://") {
		return fmt.Errorf("refusing a non-HTTPS URL: %s", url)
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return err
	}
	resp, err := installerDownloadClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("%s: %s", url, resp.Status)
	}
	f, err := os.Create(file)
	if err != nil {
		return err
	}
	n, err := io.Copy(f, io.LimitReader(resp.Body, max+1))
	if cerr := f.Close(); err == nil {
		err = cerr
	}
	if err == nil && n > max {
		err = fmt.Errorf("%s is larger than %d MiB", url, max>>20)
	}
	return err
}

// unpackReims extracts a .tar.gz into dst, refusing absolute paths, "..", links that are
// absolute or contain "..", anything written through a link, and entries other than files,
// folders and links. Every byte decompressed counts against maxReimsExpanded.
func unpackReims(r io.Reader, dst string) error {
	zr, err := gzip.NewReader(r)
	if err != nil {
		return err
	}
	tr := tar.NewReader(&budgetReader{r: zr, left: maxReimsExpanded})
	if err := os.MkdirAll(dst, 0o755); err != nil {
		return err
	}
	links := map[string]bool{}
	var total int64 // declared sizes: sparse entries expand past what the stream holds
	for n := 0; ; n++ {
		h, err := tr.Next()
		if err == io.EOF {
			return nil
		}
		if err != nil {
			return err
		}
		if n >= maxReimsEntries {
			return fmt.Errorf("the archive has more than %d entries", maxReimsEntries)
		}
		if strings.HasPrefix(h.Name, "/") || slices.Contains(strings.Split(h.Name, "/"), "..") {
			return fmt.Errorf("unexpected path %q in the archive", h.Name)
		}
		name := path.Clean(h.Name)
		if name == "." {
			if h.Typeflag != tar.TypeDir {
				return fmt.Errorf("unexpected entry %q in the archive", h.Name)
			}
			continue
		}
		for p := path.Dir(name); p != "."; p = path.Dir(p) {
			if links[p] {
				return fmt.Errorf("%q is inside the link %q", h.Name, p)
			}
		}
		target := filepath.Join(dst, filepath.FromSlash(name))
		if err := os.MkdirAll(filepath.Dir(target), 0o755); err != nil {
			return err
		}
		switch h.Typeflag {
		case tar.TypeDir:
			err = os.MkdirAll(target, 0o755)
		case tar.TypeReg:
			if total += h.Size; h.Size < 0 || total > maxReimsExpanded {
				return fmt.Errorf("the archive expands to more than %d MiB", maxReimsExpanded>>20)
			}
			err = writeFileFrom(target, tr, h.FileInfo().Mode().Perm())
		case tar.TypeSymlink:
			// Without ".." or a leading "/", a link (and any chain of them) only reaches
			// below its own folder; the packages have no other links.
			if path.IsAbs(h.Linkname) || slices.Contains(strings.Split(h.Linkname, "/"), "..") {
				return fmt.Errorf("the link %q points outside the archive (%s)", h.Name, h.Linkname)
			}
			links[name] = true
			err = os.Symlink(h.Linkname, target)
		default:
			return fmt.Errorf("unexpected entry %q in the archive (type %c)", h.Name, h.Typeflag)
		}
		if err != nil {
			return err
		}
	}
}

// budgetReader fails once more than left bytes have been read through it.
type budgetReader struct {
	r    io.Reader
	left int64
}

func (b *budgetReader) Read(p []byte) (int, error) {
	n, err := b.r.Read(p)
	if b.left -= int64(n); b.left < 0 {
		return n, fmt.Errorf("the archive expands to more than %d MiB", maxReimsExpanded>>20)
	}
	return n, err
}

// writeFileFrom creates a new file (never through an existing link) from r.
func writeFileFrom(file string, r io.Reader, perm os.FileMode) error {
	f, err := os.OpenFile(file, os.O_WRONLY|os.O_CREATE|os.O_EXCL, perm)
	if err != nil {
		return err
	}
	_, err = io.Copy(f, r)
	if cerr := f.Close(); err == nil {
		err = cerr
	}
	return err
}

// activateReims switches qemu/current to the build name when it is for the host's release and
// works here; a build for another release stays staged. Older builds of the same release go,
// unless a running QEMU uses them.
func activateReims(qdir, name, rel string, log func(string)) error {
	dir := filepath.Join(qdir, name)
	defer pruneReims(qdir, rel, name, log)
	if host, err := HostPlatform(); err != nil || host != rel {
		log(fmt.Sprintf("%s is for %s, not this host: it stays staged, not activated", dir, rel))
		return nil
	}
	if err := checkReimsBuild(dir); err != nil {
		return fmt.Errorf("%s stays staged, not activated: %w", dir, err)
	}
	tmp := filepath.Join(qdir, ".tmp-current")
	os.Remove(tmp)
	if err := os.Symlink(name, tmp); err != nil {
		return err
	}
	if err := os.Rename(tmp, filepath.Join(qdir, "current")); err != nil {
		os.Remove(tmp)
		return err
	}
	log("Reims vGPU " + name + " is active: " + filepath.Join(qdir, "current"))
	return nil
}

// checkReimsBuild runs a build's QEMU on this host: every library resolves, apple-gfx-pci is
// listed, and its option ROM is there.
func checkReimsBuild(dir string) error {
	if _, err := os.Stat(filepath.Join(dir, "share", "qemu", "ReimsVgpu.rom")); err != nil {
		return errors.New("it has no share/qemu/ReimsVgpu.rom")
	}
	bin := filepath.Join(dir, "bin", "qemu-system-x86_64")
	out, err := exec.Command(lddCommand, bin).CombinedOutput()
	if missing := lddMissing(string(out)); len(missing) > 0 {
		return missingLibsError(missing)
	}
	if err != nil {
		return fmt.Errorf("ldd %s: %v: %s", bin, err, bytes.TrimSpace(out))
	}
	return probeAppleGFX(bin)
}

// probeAppleGFX checks that a QEMU binary lists the Reims vGPU device.
func probeAppleGFX(bin string) error {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	out, err := exec.CommandContext(ctx, bin, "-device", "help").CombinedOutput()
	if err != nil {
		return fmt.Errorf("%s -device help: %v: %s", bin, err, lastLine(out))
	}
	if !bytes.Contains(out, []byte(`"apple-gfx-pci"`)) {
		return fmt.Errorf("%s does not list apple-gfx-pci: it is not a Reims vGPU build", bin)
	}
	return nil
}

// lddMissing returns the libraries ldd could not find.
func lddMissing(out string) []string {
	var missing []string
	for _, line := range strings.Split(out, "\n") {
		if lib, ok := strings.CutSuffix(strings.TrimSpace(line), " => not found"); ok {
			missing = append(missing, lib)
		}
	}
	return missing
}

// reimsLibPackages maps the libraries a Reims QEMU needs beyond a bare ubuntu image to the
// packages that ship them, the same on 24.04 and 26.04 (measured with ldd in both images).
var reimsLibPackages = map[string]string{
	"libslirp.so.0":          "libslirp0",
	"libpixman-1.so.0":       "libpixman-1-0",
	"libfdt.so.1":            "libfdt1",
	"libglib-2.0.so.0":       "libglib2.0-0t64",
	"libgio-2.0.so.0":        "libglib2.0-0t64",
	"libgobject-2.0.so.0":    "libglib2.0-0t64",
	"libgmodule-2.0.so.0":    "libglib2.0-0t64",
	"libgtk-3.so.0":          "libgtk-3-0t64",
	"libgdk-3.so.0":          "libgtk-3-0t64",
	"libgdk_pixbuf-2.0.so.0": "libgdk-pixbuf-2.0-0",
	"libcairo.so.2":          "libcairo2",
	"libepoxy.so.0":          "libepoxy0",
	"libpng16.so.16":         "libpng16-16t64",
	"libjpeg.so.8":           "libjpeg-turbo8",
	"libX11.so.6":            "libx11-6",
	"libgbm.so.1":            "libgbm1",
	"libvulkan.so.1":         "libvulkan1",
	"libpipewire-0.3.so.0":   "libpipewire-0.3-0t64",
	"libpulse.so.0":          "libpulse0",
	"libasound.so.2":         "libasound2t64",
}

func missingLibsError(missing []string) error {
	var named, pkgs []string
	for _, lib := range missing {
		if p, ok := reimsLibPackages[lib]; ok {
			named = append(named, lib+" ("+p+")")
			if !slices.Contains(pkgs, p) {
				pkgs = append(pkgs, p)
			}
		} else {
			named = append(named, lib)
		}
	}
	msg := "this host lacks libraries its QEMU needs: " + strings.Join(named, ", ")
	if len(pkgs) > 0 {
		msg += "; install them with: sudo apt install " + strings.Join(pkgs, " ")
	}
	return errors.New(msg)
}

// pruneReims removes the builds of release rel other than keep and current's, sparing any a
// running QEMU uses.
func pruneReims(qdir, rel, keep string, log func(string)) {
	cur, _ := os.Readlink(filepath.Join(qdir, "current"))
	ents, _ := os.ReadDir(qdir)
	for _, e := range ents {
		n := e.Name()
		if !e.IsDir() || n == keep || n == cur || !strings.HasSuffix(n, reimsSuffix(rel)) {
			continue
		}
		if d := filepath.Join(qdir, n); reimsInUse(d) {
			log(d + " is kept: a running VM uses it")
		} else if err := os.RemoveAll(d); err == nil {
			log("Removed the older build " + d)
		}
	}
}

// reimsInUse reports whether a running process's executable is inside dir, or whether that
// cannot be told (dir does not resolve, a QEMU process's executable is unreadable).
func reimsInUse(dir string) bool { return reimsInUseIn("/proc", dir) }

func reimsInUseIn(proc, dir string) bool {
	// /proc/<pid>/exe is canonical, so dir must be too (a Workspace may be behind a symlink).
	dir, err := filepath.EvalSymlinks(dir)
	if err != nil {
		return true
	}
	pids, _ := filepath.Glob(filepath.Join(proc, "[0-9]*"))
	for _, p := range pids {
		t, err := os.Readlink(filepath.Join(p, "exe"))
		if err != nil {
			if comm, _ := os.ReadFile(filepath.Join(p, "comm")); strings.HasPrefix(string(comm), "qemu-system") {
				return true
			}
			continue
		}
		if real, err := filepath.EvalSymlinks(t); err == nil {
			t = real
		}
		if strings.HasPrefix(t, dir+"/") {
			return true
		}
	}
	return false
}

// BuildReims builds upstream's QEMU at the Dockerfile's commit in an ubuntu:<release> container
// (release "" = the host's), caching the build in <ReimsDir>/build-<release>, then stages it and
// activates it as InstallReims does.
func BuildReims(ctx context.Context, release string, log func(string)) error {
	if release == "" {
		r, err := HostPlatform()
		if err != nil {
			return fmt.Errorf("%w: build for one of them with --release", err)
		}
		if !slices.Contains(ReimsReleases, r) {
			return fmt.Errorf("Reims vGPU builds from source for Ubuntu %s only; this host is %s: install its prebuilt package instead (Install Reims vGPU, or `macos-kvm-gui reims install`), or build an Ubuntu one with --release", strings.Join(ReimsReleases, " and "), r)
		}
		release = r
	} else if !slices.Contains(ReimsReleases, release) {
		return fmt.Errorf("Reims vGPU builds for Ubuntu %s, not %q", strings.Join(ReimsReleases, " and "), release)
	}
	if _, err := exec.LookPath("docker"); err != nil {
		return errors.New("the build needs Docker: install it (sudo apt install docker.io) and add yourself to the docker group (sudo usermod -aG docker $USER, then log in again)")
	}
	if out, err := exec.CommandContext(ctx, "docker", "info").CombinedOutput(); err != nil {
		return fmt.Errorf("Docker is not usable (%s): start it (sudo systemctl start docker) and add yourself to the docker group (sudo usermod -aG docker $USER, then log in again)", lastLine(out))
	}
	qdir, unlock, err := lockReims(ctx)
	if err != nil {
		return err
	}
	defer unlock()
	work := filepath.Join(filepath.Dir(qdir), "build-"+release)
	share := filepath.Join(work, "share", "reims-ctl", "qemu")
	os.RemoveAll(share) // only this run's build may be found there
	if err := os.MkdirAll(work, 0o755); err != nil {
		return err
	}
	image := "macos-kvm-gui-reims:" + release
	log("Preparing the build image " + image)
	if err := runLogged(ctx, log, reimsDockerfile, "docker", "build", "--progress=plain", "--build-arg", "UBUNTU="+release, "-t", image, "-"); err != nil {
		return fmt.Errorf("docker build: %w", err)
	}
	log("Building Reims vGPU for Ubuntu " + release + " (about 10 minutes the first time)")
	if err := runLogged(ctx, log, nil, "docker", "run", "--rm", "-e", "HOST_UID="+strconv.Itoa(os.Getuid()),
		"-e", "HOST_GID="+strconv.Itoa(os.Getgid()), "-v", work+":/build", image); err != nil {
		return fmt.Errorf("the build: %w", err)
	}
	built, err := filepath.EvalSymlinks(filepath.Join(share, "current"))
	if err != nil {
		return fmt.Errorf("the build left no QEMU: %w", err)
	}
	name := filepath.Base(built) + "-ubuntu" + release
	if err := os.Rename(built, filepath.Join(qdir, name)); err != nil {
		return err
	}
	return activateReims(qdir, name, release, log)
}

// runLogged runs a command and sends each output line to log. Cancelling ctx interrupts it as
// Ctrl+C would, which docker passes on to the container.
func runLogged(ctx context.Context, log func(string), stdin []byte, name string, args ...string) error {
	cmd := exec.CommandContext(ctx, name, args...)
	cmd.Cancel = func() error { return cmd.Process.Signal(os.Interrupt) }
	cmd.WaitDelay = 30 * time.Second
	cmd.Env = append(os.Environ(), "DOCKER_BUILDKIT=1")
	if stdin != nil {
		cmd.Stdin = bytes.NewReader(stdin)
	}
	pr, pw := io.Pipe()
	cmd.Stdout, cmd.Stderr = pw, pw
	if err := cmd.Start(); err != nil {
		return err
	}
	done := make(chan struct{})
	go func() {
		sc := bufio.NewScanner(pr)
		sc.Buffer(make([]byte, 64<<10), 1<<20)
		for sc.Scan() {
			log(sc.Text())
		}
		io.Copy(io.Discard, pr)
		close(done)
	}()
	err := cmd.Wait()
	pw.Close()
	<-done
	return err
}

func lastLine(out []byte) string {
	lines := strings.Split(strings.TrimSpace(string(out)), "\n")
	return lines[len(lines)-1]
}

// ReimsInfo describes the Reims QEMU in use.
type ReimsInfo struct {
	QEMU     string // the binary ReimsQEMU finds
	Build    string // its build folder's name
	Commit   string // upstream's commit, from the folder's name
	AppleGFX error  // nil when it lists apple-gfx-pci
}

// ReimsInstalled describes the Reims QEMU a reims-vgpu VM would run.
func ReimsInstalled() (ReimsInfo, error) {
	q, err := ReimsQEMU()
	if err != nil {
		return ReimsInfo{}, err
	}
	info := ReimsInfo{QEMU: q, AppleGFX: probeAppleGFX(q)}
	if real, err := filepath.EvalSymlinks(q); err == nil {
		info.Build = filepath.Base(filepath.Dir(filepath.Dir(real)))
		info.Commit = reimsCommit(info.Build)
	}
	return info, nil
}

// reimsCommit reads upstream's commit from a build name: <tag or date-time>-<commit>, with the
// platform suffix (-ubuntu<release>, -debian13, -rocky10) after it for the app's builds.
func reimsCommit(build string) string {
	cut := -1
	for _, s := range []string{"-ubuntu", "-debian", "-rocky"} {
		if i := strings.LastIndex(build, s); i > cut {
			cut = i
		}
	}
	if cut > 0 {
		build = build[:cut]
	}
	return build[strings.LastIndex(build, "-")+1:]
}

package core

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"testing"
)

func osRelease(t *testing.T, body string) {
	t.Helper()
	f := filepath.Join(t.TempDir(), "os-release")
	os.WriteFile(f, []byte(body), 0o644)
	old := osReleaseFile
	osReleaseFile = f
	t.Cleanup(func() { osReleaseFile = old })
}

func TestHostPlatform(t *testing.T) {
	for body, want := range map[string]string{
		"NAME=\"Ubuntu\"\nID=ubuntu\nVERSION_ID=\"24.04\"\n":                                                "24.04",
		"ID=ubuntu\nVERSION_ID=\"26.04\"\n":                                                                 "26.04",
		"ID=ubuntu\nVERSION_ID=\"22.04\"\nPRETTY_NAME=\"Ubuntu 22.04.5 LTS\"\n":                             "",
		"ID=debian\nVERSION_ID=\"13\"\nPRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"":                        "debian13",
		"ID=debian\nVERSION_ID=\"12\"\nPRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"":                      "",
		"ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\nVERSION_ID=\"10.2\"\n":                               "rocky10",
		"ID=\"almalinux\"\nID_LIKE=\"rhel centos fedora\"\nVERSION_ID=\"10.0\"\n":                           "rocky10",
		"ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\nVERSION_ID=\"9.6\"\nPRETTY_NAME=\"Rocky Linux 9.6\"": "",
		"ID=fedora\nVERSION_ID=42\nPRETTY_NAME=\"Fedora Linux 42\"":                                         "",
	} {
		osRelease(t, body)
		got, err := HostPlatform()
		if got != want || (want == "") != (err != nil) {
			t.Errorf("%q: %q %v", body, got, err)
		}
		if err != nil && !strings.Contains(err.Error(), "24.04 and 26.04") {
			t.Errorf("the error names the supported releases: %v", err)
		}
	}
}

func TestReimsManifest(t *testing.T) {
	m, err := loadReimsManifest()
	if err != nil {
		t.Fatal(err)
	}
	pinned := regexp.MustCompile(`ARG REIMS_COMMIT=([0-9a-f]{40})`).FindSubmatch(reimsDockerfile)
	if pinned == nil || string(pinned[1]) != m.Commit {
		t.Fatalf("the Dockerfile's commit must be the manifest's %s", m.Commit)
	}
	if m.Tag != "reims-"+m.Commit[:7] {
		t.Errorf("tag %s", m.Tag)
	}
	for _, rel := range append(slices.Clone(ReimsReleases), ReimsOtherPlatforms...) {
		p := m.Packages[rel]
		if p.File != "reims-qemu-"+m.Commit[:7]+reimsSuffix(rel)+"-x86_64.tar.gz" || !sha256Re.MatchString(p.SHA256) {
			t.Errorf("%s: %+v", rel, p)
		}
	}
}

func TestLddMissing(t *testing.T) {
	out := "\tlinux-vdso.so.1 (0x00007ffd)\n\tlibslirp.so.0 => not found\n\tlibz.so.1 => /lib/x86_64-linux-gnu/libz.so.1 (0x1)\n\tlibfoo.so.9 => not found\n"
	missing := lddMissing(out)
	if strings.Join(missing, ",") != "libslirp.so.0,libfoo.so.9" {
		t.Fatalf("%v", missing)
	}
	msg := missingLibsError(missing).Error()
	if !strings.Contains(msg, "libslirp.so.0 (libslirp0)") || !strings.Contains(msg, "libfoo.so.9") || !strings.Contains(msg, "sudo apt install libslirp0") {
		t.Fatal(msg)
	}
}

func TestReimsCommit(t *testing.T) {
	for name, want := range map[string]string{
		"20261016-212623-c099dd5a-ubuntu24.04": "c099dd5a",
		"reims-c099dd5-ubuntu26.04":            "c099dd5",
		"reims-c099dd5-debian13":               "c099dd5",
		"reims-c099dd5-rocky10":                "c099dd5",
		"20261016-212623-c099dd5a":             "c099dd5a", // upstream's own install
	} {
		if got := reimsCommit(name); got != want {
			t.Errorf("%s: %s", name, got)
		}
	}
}

type tarEntry struct {
	name, link string
	typ        byte
	body       string
}

func tarGz(t *testing.T, entries ...tarEntry) []byte {
	t.Helper()
	var b bytes.Buffer
	zw := gzip.NewWriter(&b)
	tw := tar.NewWriter(zw)
	for _, e := range entries {
		h := &tar.Header{Name: e.name, Linkname: e.link, Typeflag: e.typ, Mode: 0o755, Size: int64(len(e.body))}
		if e.typ != tar.TypeReg {
			h.Size = 0
		}
		if err := tw.WriteHeader(h); err != nil {
			t.Fatal(err)
		}
		tw.Write([]byte(e.body))
	}
	tw.Close()
	zw.Close()
	return b.Bytes()
}

func TestUnpackReims(t *testing.T) {
	good := tarGz(t,
		tarEntry{name: "./bin/", typ: tar.TypeDir},
		tarEntry{name: "./bin/qemu", typ: tar.TypeReg, body: "x"},
		tarEntry{name: "./share/qemu/a.rom", typ: tar.TypeReg, body: "rom"},
		tarEntry{name: "./share/qemu/b.rom", typ: tar.TypeSymlink, link: "a.rom"},
		tarEntry{name: "./share/top", typ: tar.TypeSymlink, link: "qemu/a.rom"})
	dst := filepath.Join(t.TempDir(), "b")
	if err := unpackReims(bytes.NewReader(good), dst); err != nil {
		t.Fatal(err)
	}
	if b, err := os.ReadFile(filepath.Join(dst, "share/qemu/b.rom")); err != nil || string(b) != "rom" {
		t.Fatalf("an inside link works: %q %v", b, err)
	}
	for why, archive := range map[string][]byte{
		"..":               tarGz(t, tarEntry{name: "bin/../../x", typ: tar.TypeReg}),
		"absolute":         tarGz(t, tarEntry{name: "/etc/x", typ: tar.TypeReg}),
		"outside link":     tarGz(t, tarEntry{name: "bin/l", typ: tar.TypeSymlink, link: "../../x"}),
		"link chain":       tarGz(t, tarEntry{name: "a", typ: tar.TypeSymlink, link: "."}, tarEntry{name: "escape", typ: tar.TypeSymlink, link: "a/.."}),
		"inside .. link":   tarGz(t, tarEntry{name: "bin/l", typ: tar.TypeSymlink, link: "../share"}),
		". as a file":      tarGz(t, tarEntry{name: ".", typ: tar.TypeReg, body: "payload"}),
		"absolute link":    tarGz(t, tarEntry{name: "l", typ: tar.TypeSymlink, link: "/etc/passwd"}),
		"through a link":   tarGz(t, tarEntry{name: "d", typ: tar.TypeSymlink, link: "."}, tarEntry{name: "d/x", typ: tar.TypeReg}),
		"hard link":        tarGz(t, tarEntry{name: "h", typ: tar.TypeLink, link: "bin/qemu"}),
		"device":           tarGz(t, tarEntry{name: "c", typ: tar.TypeChar}),
		"not a tar.gz":     []byte("plain"),
		"a file over link": tarGz(t, tarEntry{name: "l", typ: tar.TypeSymlink, link: "x"}, tarEntry{name: "l", typ: tar.TypeReg}),
	} {
		dir := t.TempDir()
		if err := unpackReims(bytes.NewReader(archive), filepath.Join(dir, "b")); err == nil {
			t.Errorf("%s: accepted", why)
		}
		if _, err := os.Lstat(filepath.Join(filepath.Dir(dir), "x")); err == nil {
			t.Errorf("%s: wrote outside", why)
		}
	}
}

func TestUnpackReimsBudget(t *testing.T) {
	old := maxReimsExpanded
	maxReimsExpanded = 4096
	t.Cleanup(func() { maxReimsExpanded = old })
	big := strings.Repeat("x", 8192)
	for why, archive := range map[string][]byte{
		"a file":     tarGz(t, tarEntry{name: "f", typ: tar.TypeReg, body: big}),
		"many files": tarGz(t, tarEntry{name: "a", typ: tar.TypeReg, body: big[:3000]}, tarEntry{name: "b", typ: tar.TypeReg, body: big[:3000]}),
	} {
		if err := unpackReims(bytes.NewReader(archive), filepath.Join(t.TempDir(), "b")); err == nil || !strings.Contains(err.Error(), "expands") {
			t.Errorf("%s: %v", why, err)
		}
	}
}

func TestReimsInUseIn(t *testing.T) {
	real := t.TempDir()
	link := filepath.Join(t.TempDir(), "ws")
	os.Symlink(real, link)
	os.MkdirAll(filepath.Join(real, "b", "bin"), 0o755)
	proc := t.TempDir()
	pid := func(n, exe, comm string) {
		os.MkdirAll(filepath.Join(proc, n), 0o755)
		if exe != "" {
			os.Symlink(exe, filepath.Join(proc, n, "exe"))
		}
		os.WriteFile(filepath.Join(proc, n, "comm"), []byte(comm+"\n"), 0o644)
	}
	pid("1", "/usr/bin/bash", "bash")
	pid("2", "", "kthreadd") // no exe: a kernel thread
	if reimsInUseIn(proc, filepath.Join(link, "b")) {
		t.Fatal("no process uses it")
	}
	pid("3", filepath.Join(real, "b", "bin", "qemu-system-x86_64"), "qemu-system-x86")
	if !reimsInUseIn(proc, filepath.Join(link, "b")) {
		t.Fatal("the canonical exe is inside the build reached through a symlink")
	}
	os.RemoveAll(filepath.Join(proc, "3"))
	pid("4", "", "qemu-system-x86") // unreadable exe
	if !reimsInUseIn(proc, filepath.Join(link, "b")) {
		t.Fatal("a QEMU whose executable cannot be read keeps the build")
	}
	if !reimsInUseIn(t.TempDir(), filepath.Join(link, "missing")) {
		t.Fatal("a build that does not resolve is kept")
	}
}

// fakeReims is a package whose QEMU is a script that lists apple-gfx-pci, with ldd faked too.
func fakeReims(t *testing.T) []byte {
	t.Helper()
	old := lddCommand
	lddCommand = "true"
	t.Cleanup(func() { lddCommand = old })
	return tarGz(t,
		tarEntry{name: "bin/qemu-system-x86_64", typ: tar.TypeReg, body: "#!/bin/sh\necho 'name \"apple-gfx-pci\", bus PCI'\n"},
		tarEntry{name: "share/qemu/ReimsVgpu.rom", typ: tar.TypeReg, body: "rom"})
}

func TestInstallReims(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("MKG_REIMS_QEMU", "")
	if err := SetWorkspaceRoot(t.TempDir()); err != nil {
		t.Fatal(err)
	}
	pkg := fakeReims(t)
	srv, _ := kextServer(t, map[string][]byte{"/reims-test/good.tar.gz": pkg})
	old, oldClient, oldURL := reimsManifestJSON, installerDownloadClient.Transport, reimsReleaseURL
	installerDownloadClient.Transport, reimsReleaseURL = srv.Client().Transport, srv.URL+"/"
	t.Cleanup(func() { reimsManifestJSON, installerDownloadClient.Transport, reimsReleaseURL = old, oldClient, oldURL })
	manifest := func(sum string) {
		reimsManifestJSON = []byte(`{"tag":"reims-test","commit":"c099dd5a4d61107a5527efc64cf1d1ab2f3a9b85","packages":{"24.04":{"file":"good.tar.gz","sha256":"` + sum + `"}}}`)
	}
	dir, _ := ReimsDir()
	current := filepath.Join(dir, "qemu", "current")
	var log []string
	logf := func(s string) { log = append(log, s) }

	osRelease(t, "ID=debian\nVERSION_ID=\"12\"\nPRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\n")
	if err := InstallReims(context.Background(), logf); err == nil || !strings.Contains(err.Error(), "24.04 and 26.04") || !strings.Contains(err.Error(), "reims build") {
		t.Fatalf("an unsupported release names the releases and the Docker build: %v", err)
	}

	osRelease(t, "ID=ubuntu\nVERSION_ID=\"24.04\"\n")
	manifest(strings.Repeat("ab", 32))
	if err := InstallReims(context.Background(), logf); err == nil || !strings.Contains(err.Error(), "SHA-256") {
		t.Fatalf("a tampered package is refused: %v", err)
	}
	if ents, _ := os.ReadDir(filepath.Join(dir, "qemu")); len(ents) != 0 {
		t.Fatalf("nothing is unpacked: %v", ents)
	}

	sum := sha256.Sum256(pkg)
	manifest(hex.EncodeToString(sum[:]))
	if err := InstallReims(context.Background(), logf); err != nil {
		t.Fatal(err)
	}
	if l, _ := os.Readlink(current); l != "reims-test-ubuntu24.04" {
		t.Fatalf("current -> %q", l)
	}
	info, err := ReimsInstalled()
	if err != nil || info.Build != "reims-test-ubuntu24.04" || info.Commit != "test" || info.AppleGFX != nil {
		t.Fatalf("%+v %v", info, err)
	}
}

// TestInstallReimsPackage installs a real package for this host, as the manifest pins it, from a
// local HTTPS server: MKG_REIMS_PACKAGES=<folder with the release's files> go test -run ...
func TestInstallReimsPackage(t *testing.T) {
	dir := os.Getenv("MKG_REIMS_PACKAGES")
	if dir == "" {
		t.Skip("MKG_REIMS_PACKAGES is not set")
	}
	rel, err := HostPlatform()
	if err != nil {
		t.Skip(err)
	}
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("MKG_REIMS_QEMU", "")
	if err := SetWorkspaceRoot(t.TempDir()); err != nil {
		t.Fatal(err)
	}
	m, _ := loadReimsManifest()
	data, err := os.ReadFile(filepath.Join(dir, m.Packages[rel].File))
	if err != nil {
		t.Fatal(err)
	}
	srv, _ := kextServer(t, map[string][]byte{"/" + m.Tag + "/" + m.Packages[rel].File: data})
	oldClient, oldURL := installerDownloadClient.Transport, reimsReleaseURL
	installerDownloadClient.Transport, reimsReleaseURL = srv.Client().Transport, srv.URL+"/"
	t.Cleanup(func() { installerDownloadClient.Transport, reimsReleaseURL = oldClient, oldURL })
	if err := InstallReims(context.Background(), func(s string) { t.Log(s) }); err != nil {
		t.Fatal(err)
	}
	info, err := ReimsInstalled()
	if err != nil || info.AppleGFX != nil || info.Build != m.Tag+"-ubuntu"+rel {
		t.Fatalf("%+v %v", info, err)
	}
	t.Logf("%+v", info)
}

func TestActivateReims(t *testing.T) {
	osRelease(t, "ID=ubuntu\nVERSION_ID=\"24.04\"\n")
	pkg := fakeReims(t)
	qdir := t.TempDir()
	build := func(name string) {
		if err := unpackReims(bytes.NewReader(pkg), filepath.Join(qdir, name)); err != nil {
			t.Fatal(err)
		}
	}
	log := func(string) {}
	build("old-ubuntu24.04")
	build("busy-ubuntu24.04")
	build("a-ubuntu26.04")
	build("new-ubuntu24.04")
	os.WriteFile(filepath.Join(qdir, "a-ubuntu26.04", "bin", "qemu-system-x86_64"), []byte("#!/bin/sh\n"), 0o755)

	// A running process inside a build keeps that build.
	sleep, _ := exec.LookPath("sleep")
	busy := filepath.Join(qdir, "busy-ubuntu24.04", "bin", "sleep")
	data, _ := os.ReadFile(sleep)
	os.WriteFile(busy, data, 0o755)
	cmd := exec.Command(busy, "30")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	defer cmd.Process.Kill()

	if err := activateReims(qdir, "a-ubuntu26.04", "26.04", log); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Lstat(filepath.Join(qdir, "current")); err == nil {
		t.Fatal("a build for another release stays staged")
	}
	if err := activateReims(qdir, "new-ubuntu24.04", "24.04", log); err != nil {
		t.Fatal(err)
	}
	var left []string
	ents, _ := os.ReadDir(qdir)
	for _, e := range ents {
		left = append(left, e.Name())
	}
	if strings.Join(left, " ") != "a-ubuntu26.04 busy-ubuntu24.04 current new-ubuntu24.04" {
		t.Fatalf("pruning: %v", left)
	}

	// A build that fails on the host leaves current as it was.
	build("broken-ubuntu24.04")
	os.Remove(filepath.Join(qdir, "broken-ubuntu24.04", "share", "qemu", "ReimsVgpu.rom"))
	if err := activateReims(qdir, "broken-ubuntu24.04", "24.04", log); err == nil || !strings.Contains(err.Error(), "staged") {
		t.Fatalf("%v", err)
	}
	if l, _ := os.Readlink(filepath.Join(qdir, "current")); l != "new-ubuntu24.04" {
		t.Fatalf("current -> %s", l)
	}
}

func TestBuildReimsRelease(t *testing.T) {
	if err := BuildReims(context.Background(), "22.04", func(string) {}); err == nil || !strings.Contains(err.Error(), "24.04 and 26.04") {
		t.Fatalf("%v", err)
	}
	osRelease(t, "ID=ubuntu\nVERSION_ID=\"22.04\"\nPRETTY_NAME=\"Ubuntu 22.04.5 LTS\"\n")
	if err := BuildReims(context.Background(), "", func(string) {}); err == nil || !strings.Contains(err.Error(), "--release") {
		t.Fatalf("an unsupported host stops before Docker: %v", err)
	}
}

func TestReimsDockerfileChownsOnExit(t *testing.T) {
	// A failed build must not leave root-owned files in the Workspace: the chown is an EXIT trap
	// set before anything is written to /build, not a last step.
	s := string(reimsDockerfile)
	trap, build := strings.Index(s, `trap 'chown -R "${HOST_UID:-0}:${HOST_GID:-0}" /build' EXIT`), strings.Index(s, "mkdir -p /build")
	if trap < 0 || build < 0 || trap > build {
		t.Fatalf("no EXIT trap before the build writes /build (trap at %d, mkdir at %d)", trap, build)
	}
}

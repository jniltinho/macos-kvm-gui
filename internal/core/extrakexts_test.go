package core

import (
	"archive/zip"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"io/fs"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"testing"
	"time"
)

// kextsYAML points the config dir at a temporary one and writes settings.yaml there.
func kextsYAML(t *testing.T, body string) {
	t.Helper()
	if os.Getenv("XDG_CONFIG_HOME") == "" || !strings.HasPrefix(os.Getenv("XDG_CONFIG_HOME"), os.TempDir()) {
		t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	}
	os.MkdirAll(filepath.Dir(SettingsFile()), 0o755)
	if err := os.WriteFile(SettingsFile(), []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestLoadExtraKexts(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	if ks, err := LoadExtraKexts(); err != nil || len(ks.Entries)+len(ks.Disabled) != 0 {
		t.Fatalf("no file is an empty list: %+v %v", ks, err)
	}
	if b, _ := os.ReadFile(SettingsFile()); !bytes.Equal(b, defaultSettingsYAML) {
		t.Fatal("the first read creates the commented default")
	}
	kextsYAML(t, "")
	if ks, err := LoadExtraKexts(); err != nil || len(ks.Entries) != 0 {
		t.Fatalf("an empty file is an empty list: %+v %v", ks, err)
	}
	kextsYAML(t, "disable: [WhateverGreen]\nkexts:\n  - url: https://example.com/a.zip\n    sha256: "+strings.Repeat("AB", 32)+"\n  - url: https://example.com/b.zip\n    load: [B, A]\n")
	ks, err := LoadExtraKexts()
	if err != nil || len(ks.Entries) != 2 || ks.Entries[0].SHA256 != strings.Repeat("ab", 32) || strings.Join(ks.Entries[1].Kexts, ":") != "B:A" || ks.Disabled[0] != "WhateverGreen" {
		t.Fatalf("valid file: %+v %v", ks, err)
	}
	for body, want := range map[string]string{
		"kexts:\n  - url: http://example.com/a.zip\n":                   "https://",
		"kexts:\n  - url: https://example.com/a.zip\n    sha256: abc\n": "64 hex",
		"kexts:\n  - url: https://example.com/a.zip\n    load: [a/b]\n": "invalid kext name",
		"kexts:\n  - url: https://example.com/a.zip\n    sha: x\n":      "not found",
		"kext:\n  - url: https://example.com/a.zip\n":                   "not found",
	} {
		kextsYAML(t, body)
		if _, err := LoadExtraKexts(); err == nil || !strings.Contains(err.Error(), want) {
			t.Errorf("%q: want %q, got %v", body, want, err)
		}
	}
}

// zipOf builds an archive; a name ending in "@" is a symlink.
func zipOf(t *testing.T, names ...string) []byte {
	var b bytes.Buffer
	z := zip.NewWriter(&b)
	for _, n := range names {
		h := &zip.FileHeader{Name: strings.TrimSuffix(n, "@"), Method: zip.Deflate}
		if strings.HasSuffix(n, "@") {
			h.SetMode(os.ModeSymlink | 0o777)
		}
		w, _ := z.CreateHeader(h)
		w.Write([]byte(n))
	}
	z.Close()
	return b.Bytes()
}

func kextFiles(names ...string) []string {
	var out []string
	for _, n := range names {
		out = append(out, n+".kext/Contents/Info.plist", n+".kext/Contents/MacOS/"+n)
	}
	return out
}

func TestCheckKextArchive(t *testing.T) {
	if got, err := checkKextArchive(zipOf(t, kextFiles("Foo")...), nil); err != nil || strings.Join(got, ",") != "Foo" {
		t.Fatalf("one kext: %v %v", got, err)
	}
	if got, err := checkKextArchive(zipOf(t, kextFiles("A", "B")...), []string{"B", "A"}); err != nil || strings.Join(got, ",") != "B,A" {
		t.Fatalf("load order: %v %v", got, err)
	}
	big := &bytes.Buffer{}
	z := zip.NewWriter(big)
	for _, n := range kextFiles("Big") {
		w, _ := z.Create(n)
		if strings.Contains(n, "MacOS") {
			w.Write(make([]byte, maxKextFile+1))
		}
	}
	z.Close()
	for name, c := range map[string]struct {
		data []byte
		want string
	}{
		"outside link":    {zipOf(t, append(kextFiles("Foo"), "elsewhere@")...), "link"},
		"dot-dot":         {zipOf(t, append(kextFiles("Foo"), "x/../../y")...), "path"},
		"absolute":        {zipOf(t, append(kextFiles("Foo"), "/etc/x")...), "path"},
		"no Info.plist":   {zipOf(t, "Foo.kext/Contents/MacOS/Foo"), "Info.plist"},
		"no kext":         {zipOf(t, "README"), "no root-level"},
		"several":         {zipOf(t, kextFiles("A", "B")...), "load: [A, B]"},
		"nested":          {zipOf(t, "dir/Foo.kext/Contents/Info.plist", "dir/Foo.kext/Contents/MacOS/Foo"), "no root-level"},
		"file over limit": {big.Bytes(), "larger than"},
	} {
		if _, err := checkKextArchive(c.data, nil); err == nil || !strings.Contains(err.Error(), c.want) {
			t.Errorf("%s: want %q, got %v", name, c.want, err)
		}
	}
	if _, err := checkKextArchive(zipOf(t, kextFiles("A")...), []string{"B"}); err == nil {
		t.Error("load naming a missing kext is refused")
	}
}

func TestUnpackKextSizeMismatch(t *testing.T) {
	var b bytes.Buffer
	z := zip.NewWriter(&b)
	w, _ := z.CreateRaw(&zip.FileHeader{Name: "Foo.kext/Contents/MacOS/Foo", Method: zip.Store, CompressedSize64: 5, UncompressedSize64: 3})
	w.Write([]byte("12345"))
	z.Close()
	if err := unpackKext(b.Bytes(), "Foo", t.TempDir()); err == nil {
		t.Fatal("a file larger than its header says is refused")
	}
}

// kextServer serves path -> archive over TLS and counts requests.
func kextServer(t *testing.T, files map[string][]byte) (*httptest.Server, *atomic.Int32) {
	var hits atomic.Int32
	srv := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits.Add(1)
		if d, ok := files[r.URL.Path]; ok {
			w.Write(d)
			return
		}
		http.NotFound(w, r)
	}))
	t.Cleanup(srv.Close)
	old := installerClient.Transport
	installerClient.Transport = srv.Client().Transport
	t.Cleanup(func() { installerClient.Transport = old })
	return srv, &hits
}

func TestHTTPSGetMax(t *testing.T) {
	srv, _ := kextServer(t, map[string][]byte{"/big": make([]byte, 11), "/ok": make([]byte, 10)})
	if _, err := httpsGetMax(context.Background(), srv.URL+"/big", 10); err == nil || !strings.Contains(err.Error(), "larger") {
		t.Errorf("past the limit: %v", err)
	}
	if d, err := httpsGetMax(context.Background(), srv.URL+"/ok", 10); err != nil || len(d) != 10 {
		t.Errorf("at the limit: %d %v", len(d), err)
	}
}

func TestDownloadExtraKexts(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	if err := SetWorkspaceRoot(t.TempDir()); err != nil {
		t.Fatal(err)
	}
	foo := zipOf(t, kextFiles("Foo")...)
	srv, hits := kextServer(t, map[string][]byte{"/foo.zip": foo})
	sum := sha256.Sum256(foo)
	e := ExtraKext{URL: srv.URL + "/foo.zip", SHA256: strings.Repeat("0", 64)}
	base, _ := ExtraKextsDir()

	if err := DownloadExtraKexts(context.Background(), ExtraKexts{Entries: []ExtraKext{e}}); err == nil || !strings.Contains(err.Error(), hex.EncodeToString(sum[:])) {
		t.Fatalf("a hash mismatch names both hashes: %v", err)
	}
	if left, _ := filepath.Glob(filepath.Join(base, "*")); len(left) != 1 || filepath.Base(left[0]) != ".lock" {
		t.Fatalf("a mismatch caches nothing: %v", left)
	}

	e.SHA256 = hex.EncodeToString(sum[:])
	ks := ExtraKexts{Entries: []ExtraKext{e}}
	hits.Store(0)
	var wg sync.WaitGroup
	for range 2 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if err := DownloadExtraKexts(context.Background(), ks); err != nil {
				t.Error(err)
			}
		}()
	}
	wg.Wait()
	if hits.Load() != 1 || !ks.Ready() {
		t.Fatalf("two starts download once: %d requests, ready=%v", hits.Load(), ks.Ready())
	}

	// A folder whose entry file names another entry is not a cache hit.
	os.WriteFile(filepath.Join(e.dir(base), "entry"), []byte("https://other\nFoo\n"), 0o644)
	if ks.Ready() {
		t.Fatal("a forged folder does not count")
	}
	if err := DownloadExtraKexts(context.Background(), ks); err != nil || !ks.Ready() || hits.Load() != 2 {
		t.Fatalf("it is downloaded again: %v", err)
	}
}

func TestExtraKextsBoot(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	root := t.TempDir()
	if err := SetWorkspaceRoot(root); err != nil {
		t.Fatal(err)
	}
	srv, _ := kextServer(t, map[string][]byte{
		"/foo.zip":  zipOf(t, kextFiles("Foo")...),
		"/pair.zip": zipOf(t, kextFiles("Plugin", "Provider")...),
		"/weg.zip":  zipOf(t, kextFiles("Whatevergreen")...),
		"/rv.zip":   zipOf(t, kextFiles("ReimsVgpu")...),
	})
	foo := ExtraKext{URL: srv.URL + "/foo.zip"}
	pair := ExtraKext{URL: srv.URL + "/pair.zip", Kexts: []string{"Provider", "Plugin"}}
	w := NewWorkspace(filepath.Join(root, "vm"))
	os.MkdirAll(w.Dir, 0o755)
	c := testDefaults()
	c.Workspace = w.Dir
	read := func() string { b, _ := os.ReadFile(w.Path(EFIDir + "/EFI/OC/config.plist")); return string(b) }

	if err := w.PrepareBoot(c, false); err != nil {
		t.Fatal(err)
	}
	before := read()
	c.Extras = &ExtraKexts{}
	if err := w.PrepareBoot(c, false); err != nil || read() != before {
		t.Fatal("an empty settings.yaml changes nothing")
	}

	c.Extras = &ExtraKexts{Entries: []ExtraKext{foo, pair}}
	if err := w.PrepareBoot(c, false); err == nil || !strings.Contains(err.Error(), "not downloaded") {
		t.Fatalf("an entry that is not cached: %v", err)
	}
	if err := w.Prepare(c); err == nil {
		t.Fatal("a start snapshot is used as given")
	}
	c2 := c
	c2.Extras = nil
	if err := w.Prepare(c2); err != nil {
		t.Fatalf("creating a VM needs no extra kext: %v", err)
	}

	if err := DownloadExtraKexts(context.Background(), *c.Extras); err != nil {
		t.Fatal(err)
	}
	c.RootPatch, c.Version = true, "sequoia"
	saved := rootPatchKexts
	defer func() { rootPatchKexts = saved }()
	rootPatchKexts = rootPatchKexts[:1]
	dir, _ := RootPatchKextsDir()
	os.MkdirAll(filepath.Join(dir, "AMFIPass.kext/Contents/MacOS"), 0o755)
	os.WriteFile(filepath.Join(dir, "AMFIPass.kext/Contents/MacOS/AMFIPass"), nil, 0o644)
	os.WriteFile(filepath.Join(dir, "AMFIPass.kext/Contents/Info.plist"), nil, 0o644)
	if err := w.PrepareBoot(c, false); err != nil {
		t.Fatal(err)
	}
	cfg := read()
	order := []string{"Lilu.kext", "VirtualSMC.kext", "WhateverGreen.kext", "AMFIPass.kext", "Foo.kext", "Provider.kext", "Plugin.kext"}
	last := -1
	for _, k := range order {
		i := strings.Index(cfg, "<string>"+k+"</string>")
		if i <= last {
			t.Fatalf("%s is out of order", k)
		}
		last = i
	}
	if !exists(w.Path(EFIDir+"/EFI/OC/Kexts/Plugin.kext/Contents/MacOS/Plugin")) || strings.Contains(cfg, "@@") {
		t.Fatal("kexts copied, every placeholder rendered")
	}
	c.RootPatch = false
	c.Extras = &ExtraKexts{}
	if err := w.PrepareBoot(c, false); err != nil || exists(w.Path(EFIDir+"/EFI/OC/Kexts/Foo.kext")) || strings.Contains(read(), "Foo.kext") {
		t.Fatal("an entry removed from settings.yaml is gone from the next boot")
	}

	weg := ExtraKext{URL: srv.URL + "/weg.zip"}
	c.Extras = &ExtraKexts{Entries: []ExtraKext{weg}}
	DownloadExtraKexts(context.Background(), *c.Extras)
	if err := w.PrepareBoot(c, false); err == nil || !strings.Contains(err.Error(), "WhateverGreen.kext") {
		t.Fatalf("a clash ignoring case names both kexts: %v", err)
	}
	c.Extras = &ExtraKexts{Entries: []ExtraKext{{URL: srv.URL + "/rv.zip"}}}
	DownloadExtraKexts(context.Background(), *c.Extras)
	if err := w.PrepareBoot(c, false); err == nil || !strings.Contains(err.Error(), "clashes with ReimsVgpu.kext") {
		t.Fatalf("ReimsVgpu is the app's own kext: %v", err)
	}
}

func TestKextSwitches(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	id, _ := newIdentity()
	c := testDefaults()
	c.Extras = &ExtraKexts{Disabled: []string{"WhateverGreen"}}
	cfg, err := OpenCoreConfig(c, id, false)
	if err != nil {
		t.Fatal(err)
	}
	entry := func(name string) string {
		i := strings.Index(cfg, "<string>"+name+".kext</string>\n\t\t\t\t<key>Comment</key>")
		return cfg[i : i+200]
	}
	if !strings.Contains(entry("WhateverGreen"), "<false/>") || !strings.Contains(entry("VirtualSMC"), "<true/>") || !strings.Contains(entry("Lilu"), "<true/>") {
		t.Fatal("only WhateverGreen is disabled")
	}
	for _, n := range []string{"Lilu", "Nope"} {
		c.Extras.Disabled = []string{n}
		if _, err := OpenCoreConfig(c, id, false); err == nil || !strings.Contains(err.Error(), n) {
			t.Errorf("%s: %v", n, err)
		}
	}
	old := "<key>Enabled</key>\n\t\t\t\t<true/>"
	tmpl := strings.ReplaceAll(strings.ReplaceAll(mustTemplate(t), "@@ENABLED_VirtualSMC@@", "<true/>"), "@@ENABLED_WhateverGreen@@", "<true/>")
	if !strings.Contains(tmpl, old) {
		t.Fatal("test template")
	}
	if _, err := kextSwitches(tmpl, nil); err != nil {
		t.Errorf("an override template without switches still works: %v", err)
	}
	if _, err := kextSwitches(tmpl, []string{"WhateverGreen"}); err == nil || !strings.Contains(err.Error(), "template") {
		t.Errorf("but cannot disable a kext: %v", err)
	}
}

func mustTemplate(t *testing.T) string {
	fsys, err := bootAssets()
	if err != nil {
		t.Fatal(err)
	}
	b, err := fs.ReadFile(fsys, "config.plist.tmpl")
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

func TestRootPatchEntryUnchanged(t *testing.T) {
	want := "\n\t\t\t<dict>\n\t\t\t\t<key>Arch</key>\n\t\t\t\t<string>x86_64</string>\n\t\t\t\t<key>BundlePath</key>\n\t\t\t\t<string>AMFIPass.kext</string>" +
		"\n\t\t\t\t<key>Comment</key>\n\t\t\t\t<string>AMFIPass.kext (root-patch mode, from OCLP 2.5.1)</string>\n\t\t\t\t<key>Enabled</key>\n\t\t\t\t<true/>" +
		"\n\t\t\t\t<key>ExecutablePath</key>\n\t\t\t\t<string>Contents/MacOS/AMFIPass</string>\n\t\t\t\t<key>MaxKernel</key>\n\t\t\t\t<string></string>" +
		"\n\t\t\t\t<key>MinKernel</key>\n\t\t\t\t<string>20.0.0</string>\n\t\t\t\t<key>PlistPath</key>\n\t\t\t\t<string>Contents/Info.plist</string>\n\t\t\t</dict>"
	if got := rootPatchPlist(true, false)["@@ROOTPATCHKEXTS@@"]; !strings.HasPrefix(got, want) {
		t.Fatalf("root-patch entries changed:\n%s", got)
	}
}

func TestExtraKextsEdges(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	kextsYAML(t, "kexts:\n  - url: https://example.com/a.zip\n    load: []\n")
	ks, err := LoadExtraKexts()
	if err != nil || ks.Entries[0].Kexts != nil || ks.Entries[0].id() != (ExtraKext{URL: "https://example.com/a.zip"}).id() {
		t.Fatalf("load: [] is no load list: %+v %v", ks, err)
	}
	kextsYAML(t, "kexts: []\n---\nkexts:\n  - url: https://example.com/a.zip\n")
	if _, err := LoadExtraKexts(); err == nil || !strings.Contains(err.Error(), "one YAML document") {
		t.Fatalf("a second document is refused: %v", err)
	}

	if err := SetWorkspaceRoot(t.TempDir()); err != nil {
		t.Fatal(err)
	}
	srv, _ := kextServer(t, map[string][]byte{"/foo.zip": zipOf(t, kextFiles("Foo")...)})
	ks = ExtraKexts{Entries: []ExtraKext{{URL: srv.URL + "/foo.zip"}}}
	if err := DownloadExtraKexts(context.Background(), ks); err != nil || !ks.Ready() {
		t.Fatal(err)
	}
	base, _ := ExtraKextsDir()
	os.Remove(filepath.Join(ks.Entries[0].dir(base), "Foo.kext", "Contents", "Info.plist"))
	if ks.Ready() {
		t.Fatal("a cached kext without Info.plist is not a hit")
	}

	// A start waiting for another one's download gives up when cancelled.
	f, _ := os.OpenFile(filepath.Join(base, ".lock"), os.O_RDWR, 0)
	defer f.Close()
	syscall.Flock(int(f.Fd()), syscall.LOCK_EX)
	ctx, cancel := context.WithTimeout(context.Background(), 300*time.Millisecond)
	defer cancel()
	if err := DownloadExtraKexts(ctx, ks); err == nil {
		t.Fatal("cancelled while another start holds the cache")
	}
}

package core

import (
	"archive/zip"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"
)

func TestKeplerAllowlist(t *testing.T) {
	for _, id := range []string{"10de:128b", "10de:1287", "10de:1180", "10de:0fc6", "10de:1004", "10de:102d"} {
		if !IsKepler(id) {
			t.Errorf("%s is Kepler (GK208, GK104, GK107, GK110, GK210)", id)
		}
	}
	// GT 730 GF108 is Fermi; GTX 745/750/750 Ti GM107 are Maxwell; AMD and Intel are not NVIDIA.
	for _, id := range []string{"10de:0f02", "10de:1380", "10de:1381", "10de:1382", "10de:2786", "1002:128b", "8086:1287"} {
		if IsKepler(id) {
			t.Errorf("%s is not Kepler", id)
		}
	}
	if len(keplerIDs) < 150 {
		t.Errorf("OCLP 2.5.1 lists about 160 Kepler IDs, got %d", len(keplerIDs))
	}
	for v, want := range map[string]bool{"big-sur": false, "monterey": true, "ventura": true, "sonoma": true, "sequoia": true, "tahoe": false, "catalina": false} {
		if RootPatchVersion(v) != want {
			t.Errorf("%s: root-patch mode %v", v, !want)
		}
		c := testDefaults()
		c.Version, c.RootPatch = v, true
		if err := c.Validate(); (err == nil) != want {
			t.Errorf("%s: Validate = %v", v, err)
		}
	}
}

func TestRootPatchConfig(t *testing.T) {
	id, _ := newIdentity()
	c := testDefaults()
	c.Version = "sequoia"
	off, err := OpenCoreConfig(c, id, false)
	if err != nil {
		t.Fatal(err)
	}
	c.RootPatch = true
	on, _ := OpenCoreConfig(c, id, false)
	c.RootPatch, c.rootPatchWasOn = false, true
	clear, _ := OpenCoreConfig(c, id, false)

	// Never turned on: the configuration of earlier versions (full SIP, no extra kexts, the
	// csr-active-config a user set with csrutil kept).
	for _, s := range []string{"AMFIPass", "revblock", "<string>csr-active-config</string>", "ipc_control_port_options", "AwgAAA=="} {
		if strings.Contains(off, s) {
			t.Errorf("default config contains %q", s)
		}
	}
	if !strings.Contains(off, "<key>csr-active-config</key>\n\t\t\t\t<data>\n\t\t\t\tAAAAAA==") {
		t.Error("default SIP is fully on")
	}
	// On: OCLP's Kepler profile, kexts after Lilu and WhateverGreen.
	for _, s := range []string{"AwgAAA==", "keepsyms=1 ipc_control_port_options=0 -nokcmismatchpanic", "<key>revblock</key>\n\t\t\t\t<string>media</string>",
		"<string>csr-active-config</string>", "<string>revblock</string>"} {
		if !strings.Contains(on, s) {
			t.Errorf("root-patch config lacks %q", s)
		}
	}
	last := strings.Index(on, "<string>WhateverGreen.kext</string>")
	for _, k := range rootPatchKexts {
		i := strings.Index(on, "<string>"+k.Name+".kext</string>")
		if i < last || !strings.Contains(on[i:], "<string>"+k.MinKernel+"</string>") {
			t.Errorf("%s must follow the other kexts with MinKernel %s", k.Name, k.MinKernel)
		}
		last = i
	}
	if strings.Contains(on, "amfi=") || strings.Contains(on, "amfi_get_out_of_my_way") {
		t.Error("AMFIPass replaces disabling AMFI")
	}
	// Turned off again: the lowered values are deleted once, nothing else is added.
	if strings.Contains(clear, "AMFIPass") || strings.Contains(clear, "AwgAAA==") || strings.Contains(clear, "<key>revblock</key>") ||
		!strings.Contains(clear, "<string>csr-active-config</string>") || !strings.Contains(clear, "<string>revblock</string>") {
		t.Error("turning the mode off deletes the lowered SIP and revblock, and adds nothing")
	}
	if v, err := filepath.Abs("testdata/ocvalidate"); err == nil && exists(v) {
		for name, cfg := range map[string]string{"off": off, "on": on, "clear": clear} {
			f := filepath.Join(t.TempDir(), "config.plist")
			write(t, f, cfg)
			if out, err := exec.Command(v, f).CombinedOutput(); err != nil || !strings.Contains(string(out), "No issues found") {
				t.Errorf("%s: ocvalidate: %v\n%s", name, err, out)
			}
		}
	}
}

// kextZip builds an archive like OCLP's: the kext, macOS metadata and, optionally, a hostile entry.
func kextZip(t *testing.T, name string, extra ...string) []byte {
	var b bytes.Buffer
	z := zip.NewWriter(&b)
	for _, n := range append([]string{name + ".kext/Contents/Info.plist", name + ".kext/Contents/MacOS/" + name, "__MACOSX/" + name + ".kext/._x"}, extra...) {
		w, _ := z.Create(n)
		w.Write([]byte(n))
	}
	z.Close()
	return b.Bytes()
}

func TestRootPatchKextsAndTransitions(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	root := t.TempDir()
	if err := SetWorkspaceRoot(root); err != nil {
		t.Fatal(err)
	}
	zips := map[string][]byte{}
	saved := rootPatchKexts
	defer func() { rootPatchKexts = saved }()
	rootPatchKexts = nil
	for _, k := range saved {
		data := kextZip(t, k.Name)
		zips["/"+k.Zip] = data
		sum := sha256.Sum256(data)
		k.SHA256 = hex.EncodeToString(sum[:])
		rootPatchKexts = append(rootPatchKexts, k)
	}
	srv := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if d, ok := zips[r.URL.Path]; ok {
			w.Write(d)
			return
		}
		http.NotFound(w, r)
	}))
	defer srv.Close()
	oldURL, oldTr := oclpBaseURL, installerClient.Transport
	oclpBaseURL, installerClient.Transport = srv.URL+"/", srv.Client().Transport
	defer func() { oclpBaseURL, installerClient.Transport = oldURL, oldTr }()

	rootPatchKexts[0].SHA256 = strings.Repeat("0", 64)
	if err := DownloadRootPatchKexts(context.Background()); err == nil || RootPatchKextsReady() {
		t.Fatal("a kext that does not match OCLP's file is refused")
	}
	rootPatchKexts[0].SHA256 = saved[0].SHA256
	sum := sha256.Sum256(zips["/"+saved[0].Zip])
	rootPatchKexts[0].SHA256 = hex.EncodeToString(sum[:])
	if err := DownloadRootPatchKexts(context.Background()); err != nil || !RootPatchKextsReady() {
		t.Fatalf("download: %v", err)
	}
	dir, _ := RootPatchKextsDir()
	if exists(filepath.Join(dir, "__MACOSX")) {
		t.Error("only the kext is unpacked")
	}
	if err := unpackKext(kextZip(t, "Evil", "Evil.kext/../../escape"), "Evil", t.TempDir()); err == nil {
		t.Error("an entry leaving the kext is refused")
	}

	// A VM: on copies the kexts and leaves the marker; the next boot with it off deletes the
	// lowered SIP once; the one after is the default again.
	w := NewWorkspace(filepath.Join(root, "vm"))
	os.MkdirAll(w.Dir, 0o755)
	c := testDefaults()
	c.Workspace, c.Version, c.RootPatch = w.Dir, "sequoia", true
	read := func() string { b, _ := os.ReadFile(w.Path(EFIDir + "/EFI/OC/config.plist")); return string(b) }
	if err := w.PrepareBoot(c, false); err != nil {
		t.Fatal(err)
	}
	if !exists(w.Path(EFIDir+"/EFI/OC/Kexts/AMFIPass.kext/Contents/MacOS/AMFIPass")) || !exists(w.Path(rootPatchMarker)) || !strings.Contains(read(), "AwgAAA==") {
		t.Fatal("mode on: kexts in the EFI, marker, lowered SIP")
	}
	c.RootPatch = false
	if err := w.PrepareBoot(c, false); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(read(), "<string>csr-active-config</string>") || exists(w.Path(EFIDir+"/EFI/OC/Kexts/AMFIPass.kext")) {
		t.Fatal("boots with the mode off delete the lowered SIP, without the kexts")
	}
	// Writing a config (workspace create) or a QEMU that stops at once does not end it.
	w.PrepareBoot(c, false)
	w.ClearRootPatchMarker(c, 5*time.Second)
	if !exists(w.Path(rootPatchMarker)) || !strings.Contains(read(), "<string>csr-active-config</string>") {
		t.Fatal("the restoration lasts until a boot really ran")
	}
	if !slices.Contains(bundleFiles, rootPatchMarker) {
		t.Fatal("a full bundle carries a pending restoration with its NVRAM")
	}
	w.ClearRootPatchMarker(c, 2*time.Minute)
	w.PrepareBoot(c, false)
	if strings.Contains(read(), "<string>csr-active-config</string>") {
		t.Fatal("later boots are the default again")
	}
}

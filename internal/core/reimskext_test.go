package core

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestResolution(t *testing.T) {
	c := Defaults()
	var fe *FieldError
	for _, r := range append([]string{""}, Resolutions...) {
		c.Resolution = r
		if err := c.Validate(); err != nil {
			t.Errorf("%q: %v", r, err)
		}
	}
	c.Resolution = "1600x900"
	if err := c.Validate(); !errors.As(err, &fe) || fe.Field != "resolution" || !strings.Contains(err.Error(), strings.Join(Resolutions, ", ")) {
		t.Errorf("invalid resolution must list the allowed values: %v", err)
	}
	for r, want := range map[string]bool{"1920x1080": false, "2048x1152": false, "2304x1296": true, "2560x1600": true, "3840x2160": false} {
		if WideResolution(r) != want {
			t.Errorf("WideResolution(%s) = %v", r, !want)
		}
	}

	withAudio(t, "none")
	c = Defaults()
	c.Graphics = GraphicsReims
	if a := strings.Join(BuildArgs(c, NewWorkspace("/vm/a"), "/run/x/qmp.sock", false, nil), " "); !strings.Contains(a, "resolution=1920x1080 ") {
		t.Errorf("empty is 1920x1080: %s", a)
	}
	c.Resolution = "3840x2160"
	if a := strings.Join(BuildArgs(c, NewWorkspace("/vm/a"), "/run/x/qmp.sock", false, nil), " "); !strings.Contains(a, "apple-gfx-pci,id=gfx,displays=1,resolution=3840x2160 ") {
		t.Errorf("resolution: %s", a)
	}
	c.Graphics = GraphicsSVGA // ignored without Reims
	if a := strings.Join(BuildArgs(c, NewWorkspace("/vm/a"), "/run/x/qmp.sock", false, nil), " "); strings.Contains(a, "3840") {
		t.Errorf("vmware-svga ignores the resolution: %s", a)
	}
}

func TestResolutionSchema5(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	path, _ := DBPath()
	os.MkdirAll(filepath.Dir(path), 0o755)
	old, err := sql.Open("sqlite", "file:"+path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := old.Exec(schemaV1 + `ALTER TABLE vms ADD COLUMN networks TEXT NOT NULL DEFAULT '';
		ALTER TABLE vms ADD COLUMN root_patch INTEGER NOT NULL DEFAULT 0;
		ALTER TABLE vms ADD COLUMN graphics TEXT NOT NULL DEFAULT '';
		PRAGMA user_version = 4;
		INSERT INTO vms (id, name, workspace, version, ram_mib, cores, vcpus, ssh_port, graphics) VALUES ('old', 'Old', '/tmp/old', 'sequoia', 8192, 4, 8, 2222, 'reims-vgpu');`); err != nil {
		t.Fatal(err)
	}
	old.Close()

	vms, err := ListVMs()
	if err != nil || len(vms) != 1 || vms[0].Config.Resolution != "" || vms[0].Config.ReimsResolution() != "1920x1080" {
		t.Fatalf("an old VM keeps 1920x1080: %v %+v", err, vms)
	}
	db, _ := openDB()
	if err := migrate(db, 4); err != nil { // safe to repeat
		t.Fatal(err)
	}
	db.Close()

	c := vms[0].Config
	c.Resolution = "2560x1440"
	if err := SaveVM(vms[0], c); err != nil {
		t.Fatal(err)
	}
	vms, _ = ListVMs()
	if vms[0].Config.Resolution != "2560x1440" {
		t.Fatalf("resolution must persist: %+v", vms[0].Config)
	}
	out, _ := ExportVM(vms[0])
	if !strings.Contains(string(out), `"resolution": "2560x1440"`) {
		t.Fatalf("export: %s", out)
	}
	f := filepath.Join(t.TempDir(), "vm.json")
	os.WriteFile(f, out, 0o644)
	if in, err := LoadConfig(f); err != nil || in.Resolution != "2560x1440" {
		t.Fatalf("import: %+v %v", in, err)
	}
	c.Resolution = "800x600"
	if err := SaveVM(vms[0], c); err == nil {
		t.Fatal("an unknown resolution must not be saved")
	}
}

func TestReimsKext(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	root := t.TempDir()
	if err := SetWorkspaceRoot(root); err != nil {
		t.Fatal(err)
	}
	files := map[string][]byte{"/Contents/Info.plist": []byte("<plist/>"), "/Contents/MacOS/ReimsVgpu": []byte("binary")}
	saved := reimsKextFiles
	defer func() { reimsKextFiles = saved }()
	reimsKextFiles = nil
	for _, f := range saved {
		sum := sha256.Sum256(files["/"+f.Path])
		f.SHA256 = hex.EncodeToString(sum[:])
		reimsKextFiles = append(reimsKextFiles, f)
	}
	srv := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if d, ok := files[r.URL.Path]; ok {
			w.Write(d)
			return
		}
		http.NotFound(w, r)
	}))
	defer srv.Close()
	oldURL, oldTr := reimsKextBaseURL, installerClient.Transport
	reimsKextBaseURL, installerClient.Transport = srv.URL+"/", srv.Client().Transport
	defer func() { reimsKextBaseURL, installerClient.Transport = oldURL, oldTr }()

	good := reimsKextFiles[1].SHA256
	reimsKextFiles[1].SHA256 = strings.Repeat("0", 64)
	dir, _ := ReimsKextDir()
	if err := DownloadReimsKext(context.Background()); err == nil || ReimsKextReady() || exists(dir) {
		t.Fatalf("a file that does not match is refused and nothing is cached: %v", err)
	}
	if left, _ := filepath.Glob(filepath.Join(filepath.Dir(dir), ".ReimsVgpu-*")); len(left) != 0 {
		t.Fatalf("temporary folders left: %v", left)
	}
	reimsKextFiles[1].SHA256 = good
	if err := DownloadReimsKext(context.Background()); err != nil || !ReimsKextReady() {
		t.Fatalf("download: %v", err)
	}
	if !strings.HasSuffix(dir, filepath.Join(".kexts", "reims-util-b302389", "ReimsVgpu.kext")) {
		t.Fatalf("cache: %s", dir)
	}

	w := NewWorkspace(filepath.Join(root, "vm"))
	os.MkdirAll(w.Dir, 0o755)
	read := func() string { b, _ := os.ReadFile(w.Path(EFIDir + "/EFI/OC/config.plist")); return string(b) }
	kext := w.Path(EFIDir + "/EFI/OC/Kexts/ReimsVgpu.kext/Contents/MacOS/ReimsVgpu")
	for _, tc := range []struct {
		version, graphics string
		start, want       bool
	}{
		{"tahoe", GraphicsReims, true, true},
		{"tahoe", GraphicsReims, false, false}, // creating the VM: offline, no kext
		{"sequoia", GraphicsReims, true, false},
		{"tahoe", GraphicsSVGA, true, false},
	} {
		c := Defaults()
		c.Workspace, c.Version, c.Graphics = w.Dir, tc.version, tc.graphics
		if tc.start {
			c.Extras = &ExtraKexts{}
		}
		if err := w.PrepareBoot(c, false); err != nil {
			t.Fatal(err)
		}
		if got := exists(kext) && strings.Contains(read(), "<string>ReimsVgpu.kext</string>"); got != tc.want {
			t.Errorf("%+v: ReimsVgpu.kext loaded = %v", tc, got)
		}
	}

	// Order: after Lilu and the other built-in kexts, before the extra kexts; MinKernel 25.0.0.
	id, _ := newIdentity()
	c := Defaults()
	c.Version, c.Graphics, c.reimsKext = "tahoe", GraphicsReims, true
	c.extraKexts = [][2]string{{"", "Foo"}}
	cfg, err := OpenCoreConfig(c, id, false)
	if err != nil {
		t.Fatal(err)
	}
	lilu, weg := strings.Index(cfg, "<string>Lilu.kext</string>"), strings.Index(cfg, "<string>WhateverGreen.kext</string>")
	i, foo := strings.Index(cfg, "<string>ReimsVgpu.kext</string>"), strings.Index(cfg, "<string>Foo.kext</string>")
	if lilu < 0 || i < lilu || i < weg || foo < i || !strings.Contains(cfg[i:foo], "<string>25.0.0</string>") {
		t.Errorf("entry order Lilu %d, WhateverGreen %d, ReimsVgpu %d, Foo %d", lilu, weg, i, foo)
	}

}

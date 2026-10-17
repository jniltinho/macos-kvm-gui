package core

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/binary"
	"encoding/json"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

const sampleCatalog = `<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>Products</key><dict>
  <key>140-1</key><dict>
    <key>ExtendedMetaInfo</key><dict><key>InstallAssistantPackageIdentifiers</key><dict><key>SharedSupport</key><string>com.apple.pkg.InstallAssistant.macOSSequoia</string></dict></dict>
    <key>Distributions</key><dict><key>English</key><string>https://swdist.apple.com/140-1.English.dist</string></dict>
    <key>Packages</key><array>
      <dict><key>URL</key><string>https://swcdn.apple.com/x/BuildManifest.plist</string><key>Size</key><integer>10</integer></dict>
      <dict><key>URL</key><string>https://swcdn.apple.com/x/InstallAssistant.pkg</string><key>Size</key><integer>14930000000</integer>
        <key>IntegrityDataURL</key><string>https://swcdn.apple.com/x/InstallAssistant.pkg.integrityDataV1</string></dict>
    </array>
  </dict>
  <key>041-2</key><dict><key>Packages</key><array/></dict>
</dict></dict></plist>`

func TestCatalogInstallers(t *testing.T) {
	got, err := catalogInstallers([]byte(sampleCatalog))
	if err != nil || len(got) != 1 {
		t.Fatalf("%v %+v", err, got)
	}
	p := got[0]
	if p.ProductID != "140-1" || p.Size != 14930000000 || !strings.HasSuffix(p.IntegrityURL, ".integrityDataV1") || !strings.HasSuffix(p.distURL, ".dist") {
		t.Fatalf("%+v", p)
	}
	v, b := distInfo([]byte(`<dict><key>BUILD</key><string>24H32</string><key>VERSION</key><string>15.8.1</string></dict>`))
	if v != "15.8.1" || b != "24H32" {
		t.Fatalf("dist: %s %s", v, b)
	}
	if !versionLess("15.9", "15.10") || versionLess("26.7.1", "26.7") || !versionLess("14.8.9", "15.0") {
		t.Fatal("versions compare numerically")
	}
	if InstallerSupported("catalina") || !InstallerSupported("big-sur") || !InstallerSupported("tahoe") {
		t.Fatal("full installers exist from Big Sur on")
	}
}

// integrity builds method-2 integrity data for data cut into chunks of n bytes.
func integrity(data []byte, n int) []byte {
	var chunks []chunk
	for i := 0; i < len(data); i += n {
		end := min(i+n, len(data))
		chunks = append(chunks, chunk{Size: uint32(end - i), Hash: sha256.Sum256(data[i:end])})
	}
	var b bytes.Buffer
	binary.Write(&b, binary.LittleEndian, chunklistHeader{Magic: [4]byte{'C', 'N', 'K', 'L'}, HeaderSize: headerSize, FileVersion: 1,
		ChunkMethod: 1, SignatureMethod: 2, ChunkCount: uint64(len(chunks)), ChunkOffset: headerSize,
		SignatureOffset: headerSize + chunkSize*uint64(len(chunks))})
	binary.Write(&b, binary.LittleEndian, chunks)
	sum := sha256.Sum256(b.Bytes())
	return append(b.Bytes(), sum[:]...)
}

func TestIntegrityAndResume(t *testing.T) {
	payload := bytes.Repeat([]byte("macOS installer payload "), 50000) // ~1.2 MB
	srv := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.ServeContent(w, r, "InstallAssistant.pkg", time.Time{}, bytes.NewReader(payload))
	}))
	defer srv.Close()
	old := installerDownloadClient.Transport
	installerDownloadClient.Transport = srv.Client().Transport
	defer func() { installerDownloadClient.Transport = old }()

	part := filepath.Join(t.TempDir(), "InstallAssistant.pkg.part")
	os.WriteFile(part, payload[:400000], 0o644) // an interrupted download
	if err := resumeDownload(context.Background(), srv.URL, part, int64(len(payload)), nil); err != nil {
		t.Fatal(err)
	}
	if got, _ := os.ReadFile(part); !bytes.Equal(got, payload) {
		t.Fatal("the resumed file must equal the original")
	}

	cnk := integrity(payload, 65536)
	chunks, err := parseIntegrity(cnk)
	if err != nil {
		t.Fatal(err)
	}
	if err := verifyChunks(context.Background(), part, chunks, nil); err != nil {
		t.Fatal(err)
	}
	bad := append([]byte(nil), payload...)
	bad[200000] ^= 1
	os.WriteFile(part, bad, 0o644)
	if err := verifyChunks(context.Background(), part, chunks, nil); err == nil || !strings.Contains(err.Error(), "integrity check failed") {
		t.Fatalf("a changed byte must fail: %v", err)
	}
	cancelled, cancel := context.WithCancel(context.Background())
	cancel()
	if err := verifyChunks(cancelled, part, chunks, nil); err == nil || cancelled.Err() == nil {
		t.Fatal("a cancelled check is an error the caller can tell from a bad package")
	}
	cnk[40] ^= 1
	if _, err := parseIntegrity(cnk); err == nil {
		t.Fatal("tampered integrity data must be refused")
	}
}

func TestInstallerDiscAndCache(t *testing.T) {
	cfg := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", cfg)
	root := t.TempDir()
	if err := SetWorkspaceRoot(root); err != nil {
		t.Fatal(err)
	}
	dir := filepath.Join(root, ".installers", "24H32")
	os.MkdirAll(dir, 0o755)
	payload := bytes.Repeat([]byte("0123456789abcdef"), 700) // 11200 bytes: pieces of 4096, 4096, 3008
	pkg := filepath.Join(dir, installerFile)
	os.WriteFile(pkg, payload, 0o644)
	old := discPiece
	discPiece = 4096
	defer func() { discPiece = old }()
	iso := filepath.Join(dir, discFile)
	cancelled, cancel := context.WithCancel(context.Background())
	cancel()
	if err := writeInstallerDisc(cancelled, pkg, iso); err == nil || exists(iso) || !exists(pkg) {
		t.Fatal("a cancelled disc write leaves no disc and keeps the package")
	}
	if err := writeInstallerDisc(context.Background(), pkg, iso); err != nil {
		t.Fatal(err)
	}

	// Read it back the way a cd9660 driver does: PVD at sector 16 → root directory → files.
	b, _ := os.ReadFile(iso)
	pvd := b[16*2048:]
	if string(pvd[1:6]) != "CD001" || pvd[0] != 1 || strings.TrimSpace(string(pvd[40:72])) != discVolume ||
		int(binary.LittleEndian.Uint32(pvd[80:]))*2048 != len(b) || b[17*2048] != 255 {
		t.Fatal("volume descriptors")
	}
	rootLBA := binary.LittleEndian.Uint32(pvd[156+2:])
	files := map[string][]byte{}
	var pieces []byte
	for d := b[rootLBA*2048 : rootLBA*2048+2048]; d[0] != 0; d = d[d[0]:] {
		name := string(d[33 : 33+d[32]])
		lba, size := binary.LittleEndian.Uint32(d[2:]), binary.LittleEndian.Uint32(d[10:])
		if binary.BigEndian.Uint32(d[6:]) != lba || binary.BigEndian.Uint32(d[14:]) != size {
			t.Fatalf("%q: both-endian fields differ", name)
		}
		files[name] = b[lba*2048 : lba*2048+size]
		if strings.HasPrefix(name, "PKG.") {
			pieces = append(pieces, files[name]...)
		}
	}
	if len(files) != 6 || files["PKG.002;1"] == nil || !bytes.Equal(pieces, payload) {
		t.Fatalf("files %d, the pieces must join into the package", len(files))
	}
	if sh := string(files["INSTALL.SH;1"]); !strings.Contains(sh, "SharedSupport.dmg") || !strings.Contains(sh, "[Pp][Kk][Gg]") {
		t.Fatalf("script: %s", sh)
	}

	os.Remove(pkg)
	info, _ := json.Marshal(InstallerProduct{Version: "15.8.1", Build: "24H32", Size: int64(len(payload))})
	os.WriteFile(filepath.Join(dir, "info.json"), info, 0o644)
	c, ok := CachedInstallerFor("sequoia")
	if !ok || c.Path != iso {
		t.Fatalf("cache: %+v %v", c, ok)
	}
	if _, ok := CachedInstallerFor("sonoma"); ok {
		t.Fatal("a Sequoia installer is not a Sonoma one")
	}
	// A disc from an older version, with another script, gets this version's head in place.
	f, _ := os.OpenFile(iso, os.O_RDWR, 0)
	f.WriteAt([]byte("#!/bin/sh\necho old\n"), 21*2048)
	f.Close()
	if err := RefreshInstallerDisc(c); err != nil {
		t.Fatal(err)
	}
	if got, _ := os.ReadFile(iso); !bytes.Equal(got, b) {
		t.Fatal("the refreshed disc must equal a new one")
	}
	cf := testDefaults()
	cf.Version = "sequoia"
	if args := strings.Join(BuildArgs(cf, Workspace{Dir: t.TempDir()}, "/q", true, nil), " "); !strings.Contains(args, "snapshot=on,file="+iso) {
		t.Errorf("the disc is attached with install media: %s", args)
	}
	done, _ := useDisc(iso)
	if err := RemoveInstaller("24H32"); err == nil {
		t.Error("an attached installer must not be deleted")
	}
	done()
	if err := RemoveInstaller("24H32"); err != nil || exists(dir) {
		t.Fatalf("remove after the VM stops: %v", err)
	}
	outside := t.TempDir()
	os.WriteFile(filepath.Join(outside, "keep"), nil, 0o644)
	os.Symlink(outside, filepath.Join(root, ".installers", "24H33"))
	for _, b := range []string{"..", ".", "a/../..", "../x", outside, "24H33", ""} {
		if err := RemoveInstaller(b); err == nil {
			t.Errorf("RemoveInstaller(%q) must fail", b)
		}
	}
	if !exists(filepath.Join(outside, "keep")) || !exists(filepath.Join(root, ".installers")) {
		t.Fatal("a rejected build deleted something")
	}
	if !buildRe.MatchString("25A5279m") || buildRe.MatchString("24H32/..") {
		t.Fatal("buildRe")
	}
}

func TestGuestKeys(t *testing.T) {
	for _, line := range []string{DiscTyped, DiscTyped + " -u"} {
		chords, err := GuestKeys(line)
		if err != nil || len(chords) != len(line)+1 {
			t.Fatalf("%q: %v", line, err)
		}
	}
	c, _ := GuestKeys("V/")
	if len(c[0]) != 2 || c[0][0] != keyCodes["LeftShift"] || c[1][0].Scancode != 0xb5 || c[2][0] != keyCodes["Return"] {
		t.Fatalf("upper case adds Shift and / is the keypad's: %v", c)
	}
	for _, c := range RecoveryTerminal() {
		for _, k := range c {
			if k.Scancode == 0 {
				t.Fatalf("the Terminal menu path has an empty key: %v", RecoveryTerminal())
			}
		}
	}
	for _, bad := range []string{"a", "m", "sh /Volumes", "x.y"} {
		if _, err := GuestKeys(bad); err == nil {
			t.Errorf("%q moves between layouts and must be refused", bad)
		}
	}
}

func TestCheckRunDir(t *testing.T) {
	base := t.TempDir()
	good := filepath.Join(base, "macos-kvm-gui-123")
	os.Mkdir(good, 0o700)
	ln, err := net.Listen("unix", filepath.Join(good, "vnc.sock"))
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	if err := CheckRunDir(filepath.Join(good, "vnc.sock")); err != nil {
		t.Fatalf("a private run directory: %v", err)
	}
	os.WriteFile(filepath.Join(good, "file.sock"), nil, 0o600)
	os.Symlink("/tmp/elsewhere.sock", filepath.Join(good, "link.sock"))
	for _, bad := range []string{"file.sock", "link.sock", "missing.sock"} {
		if err := CheckRunDir(filepath.Join(good, bad)); err == nil {
			t.Errorf("%s is not a socket of this user and must be refused", bad)
		}
	}
	open := filepath.Join(base, "macos-kvm-gui-456")
	os.Mkdir(open, 0o755)
	os.Chmod(open, 0o755)
	other := filepath.Join(base, "elsewhere")
	os.Mkdir(other, 0o700)
	link := filepath.Join(base, "macos-kvm-gui-789")
	os.Symlink(good, link)
	for _, bad := range []string{filepath.Join(open, "vnc.sock"), filepath.Join(other, "vnc.sock"), filepath.Join(link, "vnc.sock")} {
		if err := CheckRunDir(bad); err == nil {
			t.Errorf("%s must be refused", bad)
		}
	}
	args := BuildArgs(testDefaults(), Workspace{Dir: t.TempDir()}, filepath.Join(good, "qmp.sock"), false, nil)
	if !strings.Contains(strings.Join(args, " "), filepath.Join(good, "qmp-control.sock")) {
		t.Error("every run has a second QMP socket for one other process")
	}
}

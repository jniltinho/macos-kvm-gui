package core

import (
	"bytes"
	"context"
	"crypto/rand"
	"crypto/rsa"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// readyWorkspace is an existing, empty workspace directory.
func readyWorkspace(t *testing.T) Workspace { return NewWorkspace(t.TempDir()) }

func TestLockOutsideWorkspace(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	w := readyWorkspace(t)
	unlock, err := w.Lock()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := w.Lock(); !errors.Is(err, ErrBusy) {
		t.Fatalf("second acquisition must return ErrBusy: %v", err)
	}
	if entries, _ := os.ReadDir(w.Dir); len(entries) != 0 {
		t.Fatal("the lock file must live outside the workspace")
	}
	unlock()
	unlock2, err := w.Lock()
	if err != nil {
		t.Fatalf("after release it must succeed again: %v", err)
	}
	unlock2()
}

func TestPrepareExtractsBootFiles(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	w := NewWorkspace(filepath.Join(t.TempDir(), "new"))
	if w.Exists() {
		t.Fatal("must not exist yet")
	}
	if err := w.Prepare(Defaults()); err != nil {
		t.Fatal(err)
	}
	for _, f := range []string{EFIDir + "/EFI/BOOT/BOOTx64.efi", EFIDir + "/EFI/OC/OpenCore.efi", EFIDir + "/EFI/OC/config.plist",
		EFIDir + "/EFI/OC/Kexts/Lilu.kext/Contents/MacOS/Lilu", EFIDir + "/EFI/OC/Drivers/OpenRuntime.efi", OVMFCode, VarsFile, identityFile} {
		if !exists(w.Path(f)) {
			t.Errorf("%s was not extracted", f)
		}
	}
	id1, _ := os.ReadFile(w.Path(identityFile))
	write(t, w.Path(VarsFile), "modified-by-guest")
	if err := w.PrepareBoot(Defaults(), false); err != nil {
		t.Fatal(err)
	}
	if id2, _ := os.ReadFile(w.Path(identityFile)); !bytes.Equal(id1, id2) {
		t.Error("the SMBIOS identity must be generated once")
	}
	if b, _ := os.ReadFile(w.Path(VarsFile)); string(b) != "modified-by-guest" {
		t.Error("the NVRAM must never be overwritten")
	}
}

func TestOpenCoreConfig(t *testing.T) {
	id, err := newIdentity()
	if err != nil || len(id.Serial) != 12 || len(id.MLB) != 17 || len(id.UUID) != 36 || len(id.ROM) != 12 {
		t.Fatalf("identity: %+v %v", id, err)
	}
	c := Defaults()
	install, _ := OpenCoreConfig(c, id, true)
	installed, _ := OpenCoreConfig(c, id, false)
	c.GPU = "0000:03:00.0"
	gpu, _ := OpenCoreConfig(c, id, false)
	for name, cfg := range map[string]string{"install": install, "installed": installed, "gpu": gpu} {
		if strings.Contains(cfg, "@@") {
			t.Errorf("%s: unreplaced placeholder", name)
		}
		if !strings.Contains(cfg, "<string>iMac20,1</string>") || !strings.Contains(cfg, "<string>"+id.Serial+"</string>") || !strings.Contains(cfg, "1920x1080@32") {
			t.Errorf("%s: SMBIOS or resolution missing", name)
		}
	}
	if !strings.Contains(install, "<key>ShowPicker</key>\n\t\t\t<true/>") || !strings.Contains(install, "<integer>0</integer>") {
		t.Error("with install media the picker must show and everything must be scanned")
	}
	if !strings.Contains(installed, "<key>ShowPicker</key>\n\t\t\t<false/>") || !strings.Contains(installed, "<integer>257</integer>") {
		t.Error("without media the picker is hidden and only APFS is scanned")
	}
	if !strings.Contains(gpu, "keepsyms=1 agdpmod=pikera") || strings.Contains(installed, "agdpmod") {
		t.Error("agdpmod=pikera only with a passed-through GPU")
	}
	if smbiosProduct("high-sierra") != "iMac18,3" || smbiosProduct("mojave") != "iMac19,1" || smbiosProduct("tahoe") != "iMac20,1" {
		t.Error("SMBIOS per version")
	}
	// OpenCore's own validator, when scripts/fetch-assets.sh left it in testdata.
	if v, err := filepath.Abs("testdata/ocvalidate"); err == nil && exists(v) {
		for name, cfg := range map[string]string{"install": install, "installed": installed, "gpu": gpu} {
			f := filepath.Join(t.TempDir(), "config.plist")
			write(t, f, cfg)
			if out, err := exec.Command(v, f).CombinedOutput(); err != nil || !strings.Contains(string(out), "No issues found") {
				t.Errorf("%s: ocvalidate: %v\n%s", name, err, out)
			}
		}
	}
}

func TestConvertProgress(t *testing.T) {
	// qemu-img -p rewrites the line with \r.
	data := []byte("    (0.00/100%)\r    (42.50/100%)\r    (100.00/100%)\n")
	var got []float64
	for len(data) > 0 {
		n, tok, _ := scanCRLF(data, true)
		data = data[n:]
		if pct, ok := parseConvertProgress(strings.TrimSpace(string(tok))); ok {
			got = append(got, pct)
		}
	}
	if len(got) != 3 || got[1] != 42.5 || got[2] != 100 {
		t.Fatalf("got %v", got)
	}
}

func TestDownloadFailuresPreserveMedia(t *testing.T) {
	if _, err := exec.LookPath("qemu-img"); err != nil {
		t.Skip("qemu-img missing")
	}
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	key, _ := rsa.GenerateKey(rand.Reader, 2048)
	w := readyWorkspace(t)
	w.key = &key.PublicKey
	write(t, w.Path(MediaFile), "previous-media")
	write(t, w.Path(mediaMarker), "monterey\n")
	img := bytes.Repeat([]byte("not-a-dmg"), 500)
	noop := func(Progress) {}
	srv := newFakeRecovery(t)

	// 1) tampered image: fails verification, nothing converted.
	srv.chunklist, srv.dmg = string(buildChunklist(t, key, img, 1024)), "X"+string(img[1:])
	if err := w.DownloadMedia(context.Background(), "sonoma", noop, nil); err == nil || !strings.Contains(err.Error(), "verification") {
		t.Fatalf("expected verification failure: %v", err)
	}
	// 2) intact image that is not a DMG: fails conversion.
	srv.dmg = string(img)
	if err := w.DownloadMedia(context.Background(), "sonoma", noop, nil); err == nil || !strings.Contains(err.Error(), "conversion") {
		t.Fatalf("expected conversion failure: %v", err)
	}
	if b, _ := os.ReadFile(w.Path(MediaFile)); string(b) != "previous-media" {
		t.Fatal("the previous media must stay intact")
	}
	if w.MediaVersion() != "monterey" {
		t.Fatalf("the previous marker must stay intact: %q", w.MediaVersion())
	}
	if exists(w.Path(stagingDir)) {
		t.Fatal("staging must be removed")
	}
}

func TestDiskAndMediaRemoval(t *testing.T) {
	if _, err := exec.LookPath("qemu-img"); err != nil {
		t.Skip("qemu-img missing")
	}
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	w := readyWorkspace(t)
	ctx := context.Background()
	if _, err := w.CreateDisk(ctx, 39); err == nil {
		t.Fatal("39 GiB must be refused")
	}
	if _, err := w.CreateDisk(ctx, 80); err != nil {
		t.Fatal(err)
	}
	if a := Allocated(w.Path(DiskFile)); a > 10<<20 {
		t.Fatalf("thin qcow2 must allocate < 10 MiB, allocated %d", a)
	}
	out, _ := exec.Command("qemu-img", "info", w.Path(DiskFile)).Output()
	if !strings.Contains(string(out), "80 GiB") {
		t.Fatalf("virtual size: %s", out)
	}
	before, _ := os.ReadFile(w.Path(DiskFile))
	if _, err := w.CreateDisk(ctx, 100); err == nil {
		t.Fatal("must not overwrite an existing disk")
	}
	if after, _ := os.ReadFile(w.Path(DiskFile)); !bytes.Equal(before, after) {
		t.Fatal("existing disk was modified")
	}

	write(t, w.Path(MediaFile), strings.Repeat("m", 1<<20))
	freed, err := w.RemoveMedia()
	if err != nil || freed < 1<<20 || w.HasMedia() {
		t.Fatalf("removal: freed=%d err=%v", freed, err)
	}
	unlock, _ := w.Lock()
	defer unlock()
	write(t, w.Path(MediaFile), "m")
	if _, err := w.RemoveMedia(); !errors.Is(err, ErrBusy) {
		t.Fatalf("removal must be refused while the workspace is in use: %v", err)
	}
}

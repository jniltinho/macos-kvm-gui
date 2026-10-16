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

func readyWorkspace(t *testing.T) Workspace {
	w := NewWorkspace(t.TempDir())
	for _, f := range RequiredFiles {
		write(t, w.Path(f), "x")
	}
	return w
}

func TestLockOutsideWorkspace(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	dir := filepath.Join(t.TempDir(), "empty")
	os.Mkdir(dir, 0o755)
	w := NewWorkspace(dir)
	unlock, err := w.Lock()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := w.Lock(); !errors.Is(err, ErrBusy) {
		t.Fatalf("second acquisition must return ErrBusy: %v", err)
	}
	if st, _ := w.Classify(); st != WorkspaceEmpty {
		t.Fatal("the lock must not touch the workspace (git clone needs an empty directory)")
	}
	unlock()
	unlock2, err := w.Lock()
	if err != nil {
		t.Fatalf("after release it must succeed again: %v", err)
	}
	unlock2()
}

func TestClassify(t *testing.T) {
	if st, _ := NewWorkspace(filepath.Join(t.TempDir(), "does-not-exist")).Classify(); st != WorkspaceEmpty {
		t.Error("missing must be empty")
	}
	if st, _ := readyWorkspace(t).Classify(); st != WorkspaceReady {
		t.Error("complete must be ready")
	}
	w := NewWorkspace(t.TempDir())
	write(t, w.Path("something-else.txt"), "x")
	st, missing := w.Classify()
	if st != WorkspaceIncomplete || len(missing) != len(RequiredFiles) {
		t.Errorf("unrelated: %v %v", st, missing)
	}
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	if err := w.Prepare(context.Background(), nil); err == nil || !strings.Contains(err.Error(), "missing") {
		t.Errorf("prepare must not touch a non-empty directory: %v", err)
	}
}

func TestParseProgress(t *testing.T) {
	// Real fetch-macOS-v2.py output: everything on one "line" rewritten with \r.
	raw := "Saving http://x/BaseSystem.chunklist to ./BaseSystem.chunklist...\r\nDownload complete!\n" +
		"Saving http://x/BaseSystem.dmg to ./BaseSystem.dmg...\r" +
		"1.0/753.0 MB |   | 0.1% downloaded\r376.5/753.0 MB |==  | 50.0% downloaded\r"
	stage := StageChunklist
	var got []Progress
	data := []byte(raw)
	for len(data) > 0 {
		n, tok, _ := scanCRLF(data, true)
		data = data[n:]
		if p, ok := parseProgress(stage, strings.TrimSpace(string(tok))); ok {
			stage = p.Stage
			got = append(got, p)
		}
	}
	want := []Progress{{StageChunklist, -1}, {StageImage, -1}, {StageImage, 0.1}, {StageImage, 50}}
	if len(got) != len(want) {
		t.Fatalf("got %v", got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Errorf("%d: got %v want %v", i, got[i], want[i])
		}
	}
	if p, ok := parseProgress(StageConvert, "    (42.50/100%)"); !ok || p.Pct != 42.5 {
		t.Errorf("qemu-img progress: %v %v", p, ok)
	}
}

// fakeFetch replaces fetch-macOS-v2.py with a script that copies files prepared by the test.
func fakeFetch(t *testing.T, w Workspace, dmg, cnk []byte) {
	src := t.TempDir()
	write(t, filepath.Join(src, "BaseSystem.dmg"), string(dmg))
	write(t, filepath.Join(src, "BaseSystem.chunklist"), string(cnk))
	write(t, w.Path(FetchScript), "import shutil,sys\n"+
		"print('Saving http://x/BaseSystem.chunklist to ./BaseSystem.chunklist...')\n"+
		"shutil.copy('"+src+"/BaseSystem.chunklist', '.')\n"+
		"print('Saving http://x/BaseSystem.dmg to ./BaseSystem.dmg...')\n"+
		"shutil.copy('"+src+"/BaseSystem.dmg', '.')\n")
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

	// 1) tampered image: fails verification, nothing converted.
	fakeFetch(t, w, append([]byte("X"), img[1:]...), buildChunklist(t, key, img, 1024))
	if err := w.DownloadMedia(context.Background(), "sonoma", noop, nil); err == nil || !strings.Contains(err.Error(), "verification") {
		t.Fatalf("expected verification failure: %v", err)
	}
	// 2) intact image that is not a DMG: fails conversion.
	fakeFetch(t, w, img, buildChunklist(t, key, img, 1024))
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

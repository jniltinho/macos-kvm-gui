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
	dir := filepath.Join(t.TempDir(), "vazio")
	os.Mkdir(dir, 0o755)
	w := NewWorkspace(dir)
	unlock, err := w.Lock()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := w.Lock(); !errors.Is(err, ErrBusy) {
		t.Fatalf("segunda aquisição deve dar ErrBusy: %v", err)
	}
	if st, _ := w.Classify(); st != WorkspaceEmpty {
		t.Fatal("o lock não pode sujar o workspace (o git clone exige diretório vazio)")
	}
	unlock()
	unlock2, err := w.Lock()
	if err != nil {
		t.Fatalf("após soltar deve conseguir de novo: %v", err)
	}
	unlock2()
}

func TestClassify(t *testing.T) {
	if st, _ := NewWorkspace(filepath.Join(t.TempDir(), "nao-existe")).Classify(); st != WorkspaceEmpty {
		t.Error("inexistente deve ser vazio")
	}
	if st, _ := readyWorkspace(t).Classify(); st != WorkspaceReady {
		t.Error("completo deve ser pronto")
	}
	w := NewWorkspace(t.TempDir())
	write(t, w.Path("outra-coisa.txt"), "x")
	st, missing := w.Classify()
	if st != WorkspaceIncomplete || len(missing) != len(RequiredFiles) {
		t.Errorf("não relacionado: %v %v", st, missing)
	}
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	if err := w.Prepare(context.Background(), nil); err == nil || !strings.Contains(err.Error(), "faltam") {
		t.Errorf("prepare não pode mexer em diretório não vazio: %v", err)
	}
}

func TestParseProgress(t *testing.T) {
	// Saída real do fetch-macOS-v2.py: tudo em uma "linha" reescrita com \r.
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
		t.Errorf("progresso do qemu-img: %v %v", p, ok)
	}
}

// fakeFetch troca o fetch-macOS-v2.py por um script que copia arquivos preparados pelo teste.
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
		t.Skip("qemu-img ausente")
	}
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	key, _ := rsa.GenerateKey(rand.Reader, 2048)
	w := readyWorkspace(t)
	w.key = &key.PublicKey
	write(t, w.Path(MediaFile), "midia-anterior")
	write(t, w.Path(mediaMarker), "monterey\n")
	img := bytes.Repeat([]byte("nao-e-dmg"), 500)
	noop := func(Progress) {}

	// 1) imagem adulterada: falha na verificação, nada convertido.
	fakeFetch(t, w, append([]byte("X"), img[1:]...), buildChunklist(t, key, img, 1024))
	if err := w.DownloadMedia(context.Background(), "sonoma", noop, nil); err == nil || !strings.Contains(err.Error(), "verificação") {
		t.Fatalf("esperava falha de verificação: %v", err)
	}
	// 2) imagem íntegra mas que não é DMG: falha na conversão.
	fakeFetch(t, w, img, buildChunklist(t, key, img, 1024))
	if err := w.DownloadMedia(context.Background(), "sonoma", noop, nil); err == nil || !strings.Contains(err.Error(), "conversão") {
		t.Fatalf("esperava falha de conversão: %v", err)
	}
	if b, _ := os.ReadFile(w.Path(MediaFile)); string(b) != "midia-anterior" {
		t.Fatal("a mídia anterior deve ficar intacta")
	}
	if w.MediaVersion() != "monterey" {
		t.Fatalf("o marcador anterior deve ficar intacto: %q", w.MediaVersion())
	}
	if exists(w.Path(stagingDir)) {
		t.Fatal("o staging deve ser removido")
	}
}

func TestDiskAndMediaRemoval(t *testing.T) {
	if _, err := exec.LookPath("qemu-img"); err != nil {
		t.Skip("qemu-img ausente")
	}
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	w := readyWorkspace(t)
	ctx := context.Background()
	if _, err := w.CreateDisk(ctx, 39); err == nil {
		t.Fatal("39 GiB deve ser recusado")
	}
	if _, err := w.CreateDisk(ctx, 80); err != nil {
		t.Fatal(err)
	}
	if a := Allocated(w.Path(DiskFile)); a > 10<<20 {
		t.Fatalf("qcow2 thin deve alocar < 10 MiB, alocou %d", a)
	}
	out, _ := exec.Command("qemu-img", "info", w.Path(DiskFile)).Output()
	if !strings.Contains(string(out), "80 GiB") {
		t.Fatalf("tamanho virtual: %s", out)
	}
	before, _ := os.ReadFile(w.Path(DiskFile))
	if _, err := w.CreateDisk(ctx, 100); err == nil {
		t.Fatal("não pode sobrescrever disco existente")
	}
	if after, _ := os.ReadFile(w.Path(DiskFile)); !bytes.Equal(before, after) {
		t.Fatal("disco existente foi alterado")
	}

	write(t, w.Path(MediaFile), strings.Repeat("m", 1<<20))
	freed, err := w.RemoveMedia()
	if err != nil || freed < 1<<20 || w.HasMedia() {
		t.Fatalf("remoção: freed=%d err=%v", freed, err)
	}
	unlock, _ := w.Lock()
	defer unlock()
	write(t, w.Path(MediaFile), "m")
	if _, err := w.RemoveMedia(); !errors.Is(err, ErrBusy) {
		t.Fatalf("com o workspace em uso a remoção deve ser recusada: %v", err)
	}
}

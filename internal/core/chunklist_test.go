package core

import (
	"bytes"
	"crypto"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"encoding/binary"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

func TestRealChunklistSignature(t *testing.T) {
	data, err := os.ReadFile("testdata/sonoma.chunklist")
	if err != nil {
		t.Fatal(err)
	}
	chunks, err := parseChunklist(data, appleEFIKey)
	if err != nil || len(chunks) == 0 {
		t.Fatalf("chunklist real da Apple deve validar: %v", err)
	}
	data[40] ^= 1 // altera um hash: a assinatura deixa de bater
	if _, err := parseChunklist(data, appleEFIKey); err == nil {
		t.Fatal("chunklist alterado deve falhar")
	}
}

// buildChunklist cria um chunklist assinado com uma chave de teste.
func buildChunklist(t *testing.T, key *rsa.PrivateKey, img []byte, size int) []byte {
	var body bytes.Buffer
	var chunks []chunk
	for off := 0; off < len(img); off += size {
		end := min(off+size, len(img))
		chunks = append(chunks, chunk{uint32(end - off), sha256.Sum256(img[off:end])})
	}
	n := uint64(len(chunks))
	h := chunklistHeader{Magic: [4]byte{'C', 'N', 'K', 'L'}, HeaderSize: headerSize, FileVersion: 1, ChunkMethod: 1,
		SignatureMethod: 1, ChunkCount: n, ChunkOffset: headerSize, SignatureOffset: headerSize + chunkSize*n}
	binary.Write(&body, binary.LittleEndian, h)
	binary.Write(&body, binary.LittleEndian, chunks)
	digest := sha256.Sum256(body.Bytes())
	sig, err := rsa.SignPKCS1v15(rand.Reader, key, crypto.SHA256, digest[:])
	if err != nil {
		t.Fatal(err)
	}
	slices.Reverse(sig)
	return append(body.Bytes(), sig...)
}

func TestVerifyImage(t *testing.T) {
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	img := bytes.Repeat([]byte("macOS"), 1000)
	dmg, cnk := filepath.Join(dir, "a.dmg"), filepath.Join(dir, "a.chunklist")
	write(t, cnk, string(buildChunklist(t, key, img, 1024)))

	cases := []struct {
		name, content, wantErr string
	}{
		{"íntegra", string(img), ""},
		{"adulterada", strings.Replace(string(img), "macOS", "MACOS", 1), "não confere"},
		{"truncada", string(img[:len(img)-10]), "truncada"},
		{"maior", string(img) + "x", "maior"},
	}
	for _, c := range cases {
		write(t, dmg, c.content)
		calls := 0
		err := verifyImage(dmg, cnk, &key.PublicKey, func(int, int) { calls++ })
		if c.wantErr == "" && (err != nil || calls == 0) {
			t.Errorf("%s: %v (progresso %d)", c.name, err, calls)
		}
		if c.wantErr != "" && (err == nil || !strings.Contains(err.Error(), c.wantErr)) {
			t.Errorf("%s: esperado erro %q, veio %v", c.name, c.wantErr, err)
		}
	}
	if err := verifyImage(dmg, cnk, appleEFIKey, nil); err == nil {
		t.Error("chunklist assinado com outra chave deve falhar com a chave da Apple")
	}
}

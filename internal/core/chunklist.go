package core

import (
	"bytes"
	"crypto"
	"crypto/rsa"
	"crypto/sha256"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"math/big"
	"os"
	"slices"
)

// appleEFIKey é a Apple_EFI_ROM_public_key_1 do fetch-macOS-v2.py (RSA-2048, e=65537).
var appleEFIKey = &rsa.PublicKey{
	N: mustHex("C3E748CAD9CD384329E10E25A91E43E1A762FF529ADE578C935BDDF9B13F2179D4855E6FC89E9E29CA12517D17DFA1EDCE0BEBF0EA7B461FFE61D94E2BDF72C196F89ACD3536B644064014DAE25A15DB6BB0852ECBD120916318D1CCDEA3C84C92ED743FC176D0BACA920D3FCF3158AFF731F88CE0623182A8ED67E650515F75745909F07D415F55FC15A35654D118C55A462D37A3ACDA08612F3F3F6571761EFCCBCC299AEE99B3A4FD6212CCFFF5EF37A2C334E871191F7E1C31960E010A54E86FA3F62E6D6905E1CD57732410A3EB0C6B4DEFDABE9F59BF1618758C751CD56CEF851D1C0EAA1C558E37AC108DA9089863D20E2E7E4BF475EC66FE6B3EFDCF"),
	E: 65537,
}

func mustHex(s string) *big.Int {
	n, ok := new(big.Int).SetString(s, 16)
	if !ok {
		panic("hex inválido")
	}
	return n
}

// chunklistHeader segue o struct '<4sIBBBxQQQ' do script (36 bytes).
type chunklistHeader struct {
	Magic           [4]byte
	HeaderSize      uint32
	FileVersion     uint8
	ChunkMethod     uint8
	SignatureMethod uint8
	_               uint8
	ChunkCount      uint64
	ChunkOffset     uint64
	SignatureOffset uint64
}

type chunk struct {
	Size uint32
	Hash [32]byte
}

const (
	headerSize = 36
	chunkSize  = 36
)

// parseChunklist valida estrutura e assinatura e devolve os chunks esperados.
func parseChunklist(data []byte, key *rsa.PublicKey) ([]chunk, error) {
	r := bytes.NewReader(data)
	var h chunklistHeader
	if err := binary.Read(r, binary.LittleEndian, &h); err != nil {
		return nil, fmt.Errorf("chunklist truncado: %w", err)
	}
	if string(h.Magic[:]) != "CNKL" || h.HeaderSize != headerSize || h.FileVersion != 1 || h.ChunkMethod != 1 ||
		h.ChunkCount == 0 || h.ChunkOffset != headerSize || h.SignatureOffset != headerSize+chunkSize*h.ChunkCount {
		return nil, errors.New("chunklist com cabeçalho inválido")
	}
	if h.SignatureMethod != 1 {
		return nil, errors.New("chunklist sem assinatura digital da Apple")
	}
	if uint64(len(data)) != h.SignatureOffset+256 {
		return nil, errors.New("chunklist com tamanho inválido")
	}
	chunks := make([]chunk, h.ChunkCount)
	if err := binary.Read(r, binary.LittleEndian, chunks); err != nil {
		return nil, err
	}
	digest := sha256.Sum256(data[:h.SignatureOffset])
	sig := slices.Clone(data[h.SignatureOffset:])
	slices.Reverse(sig) // o script lê a assinatura como inteiro little-endian
	if err := rsa.VerifyPKCS1v15(key, crypto.SHA256, digest[:], sig); err != nil {
		return nil, errors.New("assinatura do chunklist inválida")
	}
	return chunks, nil
}

// VerifyImage confere o DMG contra o chunklist assinado pela Apple.
// progress recebe (chunks conferidos, total) e pode ser nil.
func VerifyImage(dmgPath, cnkPath string, progress func(done, total int)) error {
	return verifyImage(dmgPath, cnkPath, appleEFIKey, progress)
}

func verifyImage(dmgPath, cnkPath string, key *rsa.PublicKey, progress func(done, total int)) error {
	data, err := os.ReadFile(cnkPath)
	if err != nil {
		return err
	}
	chunks, err := parseChunklist(data, key)
	if err != nil {
		return err
	}
	f, err := os.Open(dmgPath)
	if err != nil {
		return err
	}
	defer f.Close()
	buf := make([]byte, 0, 10<<20)
	for i, c := range chunks {
		buf = buf[:c.Size]
		if _, err := io.ReadFull(f, buf); err != nil {
			return fmt.Errorf("imagem truncada no chunk %d", i+1)
		}
		if sha256.Sum256(buf) != c.Hash {
			return fmt.Errorf("chunk %d não confere (imagem corrompida)", i+1)
		}
		if progress != nil {
			progress(i+1, len(chunks))
		}
	}
	if n, _ := f.Read(make([]byte, 1)); n != 0 {
		return errors.New("imagem maior que o chunklist")
	}
	return nil
}

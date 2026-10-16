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

// appleEFIKey is Apple_EFI_ROM_public_key_1 from fetch-macOS-v2.py (RSA-2048, e=65537).
var appleEFIKey = &rsa.PublicKey{
	N: mustHex("C3E748CAD9CD384329E10E25A91E43E1A762FF529ADE578C935BDDF9B13F2179D4855E6FC89E9E29CA12517D17DFA1EDCE0BEBF0EA7B461FFE61D94E2BDF72C196F89ACD3536B644064014DAE25A15DB6BB0852ECBD120916318D1CCDEA3C84C92ED743FC176D0BACA920D3FCF3158AFF731F88CE0623182A8ED67E650515F75745909F07D415F55FC15A35654D118C55A462D37A3ACDA08612F3F3F6571761EFCCBCC299AEE99B3A4FD6212CCFFF5EF37A2C334E871191F7E1C31960E010A54E86FA3F62E6D6905E1CD57732410A3EB0C6B4DEFDABE9F59BF1618758C751CD56CEF851D1C0EAA1C558E37AC108DA9089863D20E2E7E4BF475EC66FE6B3EFDCF"),
	E: 65537,
}

func mustHex(s string) *big.Int {
	n, ok := new(big.Int).SetString(s, 16)
	if !ok {
		panic("invalid hex")
	}
	return n
}

// chunklistHeader follows the script's '<4sIBBBxQQQ' struct (36 bytes).
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

// parseChunklist validates structure and signature and returns the expected chunks.
func parseChunklist(data []byte, key *rsa.PublicKey) ([]chunk, error) {
	r := bytes.NewReader(data)
	var h chunklistHeader
	if err := binary.Read(r, binary.LittleEndian, &h); err != nil {
		return nil, fmt.Errorf("chunklist truncated: %w", err)
	}
	if string(h.Magic[:]) != "CNKL" || h.HeaderSize != headerSize || h.FileVersion != 1 || h.ChunkMethod != 1 ||
		h.ChunkCount == 0 || h.ChunkCount > uint64(len(data))/chunkSize || h.ChunkOffset != headerSize || h.SignatureOffset != headerSize+chunkSize*h.ChunkCount {
		return nil, errors.New("chunklist has an invalid header")
	}
	if h.SignatureMethod != 1 {
		return nil, errors.New("chunklist has no Apple digital signature")
	}
	if uint64(len(data)) != h.SignatureOffset+256 {
		return nil, errors.New("chunklist has an invalid size")
	}
	chunks := make([]chunk, h.ChunkCount)
	if err := binary.Read(r, binary.LittleEndian, chunks); err != nil {
		return nil, err
	}
	digest := sha256.Sum256(data[:h.SignatureOffset])
	sig := slices.Clone(data[h.SignatureOffset:])
	slices.Reverse(sig) // the script reads the signature as a little-endian integer
	if err := rsa.VerifyPKCS1v15(key, crypto.SHA256, digest[:], sig); err != nil {
		return nil, errors.New("invalid chunklist signature")
	}
	return chunks, nil
}

// VerifyImage checks the DMG against the Apple-signed chunklist.
// progress receives (chunks checked, total) and may be nil.
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
			return fmt.Errorf("image truncated at chunk %d", i+1)
		}
		if sha256.Sum256(buf) != c.Hash {
			return fmt.Errorf("chunk %d does not match (corrupted image)", i+1)
		}
		if progress != nil {
			progress(i+1, len(chunks))
		}
	}
	if n, _ := f.Read(make([]byte, 1)); n != 0 {
		return errors.New("image larger than the chunklist")
	}
	return nil
}

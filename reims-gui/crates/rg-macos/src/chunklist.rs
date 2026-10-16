//! Apple's CNKL chunklists: the recovery's (RSA-signed with Apple's EFI key) and the full
//! installer's integrity data (a plain SHA-256), and checking files against them.

use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use rsa::{BigUint, Pkcs1v15Sign, RsaPublicKey};
use sha2::{Digest, Sha256};

use crate::{Error, Result, check_cancel, other};

/// Apple_EFI_ROM_public_key_1 from fetch-macOS-v2.py (RSA-2048, e=65537).
const APPLE_EFI_KEY_N: &str = "C3E748CAD9CD384329E10E25A91E43E1A762FF529ADE578C935BDDF9B13F2179D4855E6FC89E9E29CA12517D17DFA1EDCE0BEBF0EA7B461FFE61D94E2BDF72C196F89ACD3536B644064014DAE25A15DB6BB0852ECBD120916318D1CCDEA3C84C92ED743FC176D0BACA920D3FCF3158AFF731F88CE0623182A8ED67E650515F75745909F07D415F55FC15A35654D118C55A462D37A3ACDA08612F3F3F6571761EFCCBCC299AEE99B3A4FD6212CCFFF5EF37A2C334E871191F7E1C31960E010A54E86FA3F62E6D6905E1CD57732410A3EB0C6B4DEFDABE9F59BF1618758C751CD56CEF851D1C0EAA1C558E37AC108DA9089863D20E2E7E4BF475EC66FE6B3EFDCF";

pub(crate) fn apple_efi_key() -> Result<RsaPublicKey> {
    let n = BigUint::parse_bytes(APPLE_EFI_KEY_N.as_bytes(), 16)
        .ok_or_else(|| Error::Other("invalid Apple key".into()))?;
    RsaPublicKey::new(n, BigUint::from(65537u32)).map_err(|e| Error::Other(e.to_string()))
}

/// PKCS#1 v1.5 for SHA-256: the DigestInfo prefix of RFC 8017 9.2 (no second `digest` version).
fn pkcs1_sha256() -> Pkcs1v15Sign {
    const PREFIX: [u8; 19] = [
        0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01,
        0x05, 0x00, 0x04, 0x20,
    ];
    Pkcs1v15Sign {
        hash_len: Some(32),
        prefix: Box::new(PREFIX),
    }
}

/// One chunk of a file: its size and SHA-256.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// Bytes in the chunk.
    pub size: u32,
    /// SHA-256 of the chunk.
    pub hash: [u8; 32],
}

pub(crate) const HEADER_SIZE: u64 = 36;
pub(crate) const CHUNK_SIZE: u64 = 36;

/// The script's '<4sIBBBxQQQ' header (36 bytes).
pub(crate) struct Header {
    pub magic: [u8; 4],
    pub header_size: u32,
    pub file_version: u8,
    pub chunk_method: u8,
    pub signature_method: u8,
    pub chunk_count: u64,
    pub chunk_offset: u64,
    pub signature_offset: u64,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    let mut a = [0; 4];
    a.copy_from_slice(&b[at..at + 4]);
    u32::from_le_bytes(a)
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

impl Header {
    fn parse(data: &[u8]) -> Option<Self> {
        let h = data.get(..HEADER_SIZE as usize)?;
        Some(Self {
            magic: [h[0], h[1], h[2], h[3]],
            header_size: u32_at(h, 4),
            file_version: h[8],
            chunk_method: h[9],
            signature_method: h[10],
            chunk_count: u64_at(h, 12),
            chunk_offset: u64_at(h, 20),
            signature_offset: u64_at(h, 28),
        })
    }

    /// The structural checks both formats share (the file version only for the signed one).
    fn valid(&self, len: usize) -> bool {
        &self.magic == b"CNKL"
            && u64::from(self.header_size) == HEADER_SIZE
            && self.chunk_method == 1
            && self.chunk_count != 0
            && self.chunk_count <= len as u64 / CHUNK_SIZE
            && self.chunk_offset == HEADER_SIZE
            && self.signature_offset == HEADER_SIZE + CHUNK_SIZE * self.chunk_count
    }

    #[cfg(test)]
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(HEADER_SIZE as usize);
        b.extend_from_slice(&self.magic);
        b.extend_from_slice(&self.header_size.to_le_bytes());
        b.extend_from_slice(&[
            self.file_version,
            self.chunk_method,
            self.signature_method,
            0,
        ]);
        b.extend_from_slice(&self.chunk_count.to_le_bytes());
        b.extend_from_slice(&self.chunk_offset.to_le_bytes());
        b.extend_from_slice(&self.signature_offset.to_le_bytes());
        b
    }
}

fn read_chunks(data: &[u8], count: u64) -> Vec<Chunk> {
    let (chunks, _) = data[HEADER_SIZE as usize..].as_chunks::<{ CHUNK_SIZE as usize }>();
    chunks
        .iter()
        .take(count as usize)
        .map(|c| {
            let mut hash = [0; 32];
            hash.copy_from_slice(&c[4..]);
            Chunk {
                size: u32_at(c, 0),
                hash,
            }
        })
        .collect()
}

/// Validates a recovery chunklist's structure and Apple signature and returns its chunks.
pub(crate) fn parse_chunklist(data: &[u8], key: &RsaPublicKey) -> Result<Vec<Chunk>> {
    let Some(h) = Header::parse(data) else {
        return other("chunklist truncated");
    };
    if !h.valid(data.len()) || h.file_version != 1 {
        return other("chunklist has an invalid header");
    }
    if h.signature_method != 1 {
        return other("chunklist has no Apple digital signature");
    }
    if data.len() as u64 != h.signature_offset + 256 {
        return other("chunklist has an invalid size");
    }
    let signed = &data[..h.signature_offset as usize];
    let digest = Sha256::digest(signed);
    let mut sig = data[h.signature_offset as usize..].to_vec();
    sig.reverse(); // the script reads the signature as a little-endian integer
    if key.verify(pkcs1_sha256(), &digest, &sig).is_err() {
        return other("invalid chunklist signature");
    }
    Ok(read_chunks(data, h.chunk_count))
}

/// Reads InstallAssistant's integrity data: a CNKL chunklist with signature method 2, whose last
/// 32 bytes are the SHA-256 of everything before them. It is not signed by Apple (unlike the
/// recovery's chunklist); its authenticity comes from Apple's HTTPS servers.
pub(crate) fn parse_integrity(data: &[u8]) -> Result<Vec<Chunk>> {
    let Some(h) = Header::parse(data) else {
        return other("integrity data truncated");
    };
    if !h.valid(data.len()) {
        return other("integrity data has an invalid header");
    }
    if h.signature_method != 2 || data.len() as u64 != h.signature_offset + 32 {
        return other("integrity data has an unexpected checksum format");
    }
    let (body, sum) = data.split_at(h.signature_offset as usize);
    if Sha256::digest(body).as_slice() != sum {
        return other("integrity data is corrupted");
    }
    Ok(read_chunks(data, h.chunk_count))
}

/// Checks the recovery DMG against its Apple-signed chunklist: every chunk's SHA-256, and no
/// missing or trailing bytes. `progress(chunks checked, total)` is called after each chunk.
pub fn verify_image(
    dmg: &Path,
    chunklist: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    verify_image_with(dmg, chunklist, &apple_efi_key()?, progress)
}

pub(crate) fn verify_image_with(
    dmg: &Path,
    chunklist: &Path,
    key: &RsaPublicKey,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let chunks = parse_chunklist(&std::fs::read(chunklist)?, key)?;
    let mut f = File::open(dmg)?;
    let mut buf = Vec::new();
    let total = chunks.len() as u64;
    for (i, c) in chunks.iter().enumerate() {
        buf.resize(c.size as usize, 0);
        match f.read_exact(&mut buf) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                return other(format!("image truncated at chunk {}", i + 1));
            }
            Err(e) => return Err(e.into()),
        }
        if Sha256::digest(&buf).as_slice() != c.hash {
            return other(format!("chunk {} does not match (corrupted image)", i + 1));
        }
        progress(i as u64 + 1, total);
    }
    if f.read(&mut [0])? != 0 {
        return other("image larger than the chunklist");
    }
    Ok(())
}

/// Checks a file against its chunks, reporting `progress(bytes done, total)` every 64 chunks.
pub(crate) fn verify_chunks(
    path: &Path,
    chunks: &[Chunk],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let mut f = File::open(path)?;
    let total: u64 = chunks.iter().map(|c| u64::from(c.size)).sum();
    if f.metadata()?.len() != total {
        return other("package integrity check failed: wrong size");
    }
    let mut buf = Vec::new();
    let mut done = 0;
    for (i, c) in chunks.iter().enumerate() {
        check_cancel(cancel)?;
        buf.resize(c.size as usize, 0);
        if f.read_exact(&mut buf).is_err() {
            return other(format!(
                "package integrity check failed: truncated at chunk {}",
                i + 1
            ));
        }
        if Sha256::digest(&buf).as_slice() != c.hash {
            return other(format!(
                "package integrity check failed: chunk {} does not match",
                i + 1
            ));
        }
        done += u64::from(c.size);
        if i % 64 == 0 || i == chunks.len() - 1 {
            progress(done, total);
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rsa::RsaPrivateKey;
    use rsa::pkcs1::DecodeRsaPrivateKey;
    use std::sync::atomic::AtomicBool;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    pub(crate) fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    /// A fixed 2048-bit test key (generating one is slow in debug builds).
    pub(crate) fn test_key() -> std::result::Result<RsaPrivateKey, Box<dyn std::error::Error>> {
        Ok(RsaPrivateKey::from_pkcs1_der(&std::fs::read(fixture(
            "test-key.der",
        ))?)?)
    }

    fn chunks_of(img: &[u8], size: usize) -> Vec<Chunk> {
        img.chunks(size)
            .map(|c| Chunk {
                size: c.len() as u32,
                hash: Sha256::digest(c).into(),
            })
            .collect()
    }

    fn body(chunks: &[Chunk], signature_method: u8) -> Vec<u8> {
        let n = chunks.len() as u64;
        let mut b = Header {
            magic: *b"CNKL",
            header_size: HEADER_SIZE as u32,
            file_version: 1,
            chunk_method: 1,
            signature_method,
            chunk_count: n,
            chunk_offset: HEADER_SIZE,
            signature_offset: HEADER_SIZE + CHUNK_SIZE * n,
        }
        .to_bytes();
        for c in chunks {
            b.extend_from_slice(&c.size.to_le_bytes());
            b.extend_from_slice(&c.hash);
        }
        b
    }

    /// A recovery chunklist for `img` signed with `key`.
    pub(crate) fn build_chunklist(key: &RsaPrivateKey, img: &[u8], size: usize) -> Vec<u8> {
        let mut b = body(&chunks_of(img, size), 1);
        let digest = Sha256::digest(&b);
        let Ok(mut sig) = key.sign(pkcs1_sha256(), &digest) else {
            panic!("sign")
        };
        sig.reverse();
        b.extend_from_slice(&sig);
        b
    }

    /// Method-2 integrity data for `data` cut into chunks of `n` bytes.
    pub(crate) fn integrity(data: &[u8], n: usize) -> Vec<u8> {
        let mut b = body(&chunks_of(data, n), 2);
        let sum = Sha256::digest(&b);
        b.extend_from_slice(&sum);
        b
    }

    #[test]
    fn real_chunklist_signature() -> TestResult {
        let mut data = std::fs::read(fixture("sonoma.chunklist"))?;
        let key = apple_efi_key()?;
        assert!(
            !parse_chunklist(&data, &key)?.is_empty(),
            "a real Apple chunklist must validate"
        );
        data[40] ^= 1; // change a hash: the signature no longer matches
        assert!(
            parse_chunklist(&data, &key).is_err(),
            "a modified chunklist must fail"
        );
        Ok(())
    }

    #[test]
    fn verify_image_cases() -> TestResult {
        let key = test_key()?;
        let dir = tempdir()?;
        let img = b"macOS".repeat(1000);
        let (dmg, cnk) = (dir.join("a.dmg"), dir.join("a.chunklist"));
        std::fs::write(&cnk, build_chunklist(&key, &img, 1024))?;
        let tampered = String::from_utf8(img.clone())?
            .replacen("macOS", "MACOS", 1)
            .into_bytes();
        let larger = [img.as_slice(), b"x"].concat();
        let cases: [(&str, &[u8], &str); 4] = [
            ("intact", &img, ""),
            ("tampered", &tampered, "does not match"),
            ("truncated", &img[..img.len() - 10], "truncated"),
            ("larger", &larger, "larger"),
        ];
        for (name, content, want) in cases {
            std::fs::write(&dmg, content)?;
            let mut calls = 0;
            let res = verify_image_with(&dmg, &cnk, &key.to_public_key(), &mut |_, _| calls += 1);
            match res {
                Ok(()) => assert!(want.is_empty() && calls > 0, "{name}: progress {calls}"),
                Err(e) => assert!(
                    !want.is_empty() && e.to_string().contains(want),
                    "{name}: {e}"
                ),
            }
        }
        assert!(
            verify_image(&dmg, &cnk, &mut |_, _| {}).is_err(),
            "a chunklist signed with another key must fail with Apple's key"
        );
        Ok(())
    }

    #[test]
    fn chunklist_huge_count_does_not_panic() -> TestResult {
        let mut data = vec![0u8; HEADER_SIZE as usize + 256]; // 292 bytes, as the header claims
        data[..4].copy_from_slice(b"CNKL");
        data[4..8].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
        data[8..11].copy_from_slice(&[1, 1, 1]);
        data[12..20].copy_from_slice(&(1u64 << 62).to_le_bytes()); // the offset sum overflows
        data[20..28].copy_from_slice(&HEADER_SIZE.to_le_bytes());
        data[28..36].copy_from_slice(&HEADER_SIZE.to_le_bytes());
        assert!(parse_chunklist(&data, &apple_efi_key()?).is_err());
        Ok(())
    }

    #[test]
    fn integrity_checks() -> TestResult {
        let dir = tempdir()?;
        let payload = b"macOS installer payload ".repeat(50000);
        let path = dir.join("pkg");
        std::fs::write(&path, &payload)?;
        let mut cnk = integrity(&payload, 65536);
        let chunks = parse_integrity(&cnk)?;
        let never = AtomicBool::new(false);
        verify_chunks(&path, &chunks, &never, &mut |_, _| {})?;

        let mut bad = payload.clone();
        bad[200000] ^= 1;
        std::fs::write(&path, &bad)?;
        let e = verify_chunks(&path, &chunks, &never, &mut |_, _| {})
            .err()
            .map(|e| e.to_string());
        assert!(
            e.is_some_and(|e| e.contains("integrity check failed")),
            "a changed byte must fail"
        );

        let cancelled = AtomicBool::new(true);
        assert!(
            matches!(
                verify_chunks(&path, &chunks, &cancelled, &mut |_, _| {}),
                Err(Error::Cancelled)
            ),
            "a cancelled check is an error the caller can tell from a bad package"
        );
        cnk[40] ^= 1;
        assert!(
            parse_integrity(&cnk).is_err(),
            "tampered integrity data must be refused"
        );
        Ok(())
    }

    /// A fresh, empty directory under the system temp dir (unique per process and call).
    pub(crate) fn tempdir() -> std::io::Result<std::path::PathBuf> {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "rg-macos-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }
}

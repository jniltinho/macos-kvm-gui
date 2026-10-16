//! Small helpers: randomness, hex/base64, temporary folders, atomic writes, tree copies.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::Result;

/// Bytes from the kernel's CSPRNG (Linux only, like the app).
pub(crate) fn random_bytes(n: usize) -> std::io::Result<Vec<u8>> {
    let mut b = vec![0; n];
    fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b)
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

/// Decodes lowercase or uppercase hex; `None` when it is not hex.
pub(crate) fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Standard base64 with padding (Go's `base64.StdEncoding`).
pub(crate) fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(char::from(A[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A folder removed (with its contents) when dropped, like Go's `MkdirTemp` + `defer RemoveAll`.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    /// Creates `<parent>/<prefix><random>`.
    pub(crate) fn new_in(parent: &Path, prefix: &str) -> std::io::Result<Self> {
        loop {
            let p = parent.join(format!("{prefix}{}", hex(&random_bytes(6)?)));
            match fs::create_dir(&p) {
                Ok(()) => return Ok(Self(p)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0); // gone already when it was renamed into place
    }
}

/// Copies a folder of regular files; links and other special files are refused.
pub(crate) fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    let ty = fs::symlink_metadata(src)?.file_type();
    if ty.is_dir() {
        fs::create_dir_all(dst)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_tree(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else if ty.is_file() {
        fs::copy(src, dst)?;
        Ok(())
    } else {
        crate::invalid(format!("unexpected file {}", src.display()))
    }
}

/// Go's `exists`: anything `stat` finds.
pub(crate) fn exists(p: &Path) -> bool {
    fs::metadata(p).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_go_std_encoding() {
        for (i, o) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("en-US:128", "ZW4tVVM6MTI4"),
        ] {
            assert_eq!(base64(i.as_bytes()), o);
        }
        assert_eq!(base64(&[0x03, 0x08, 0, 0]), "AwgAAA==");
    }

    #[test]
    fn hex_round_trips() {
        assert_eq!(hex_decode("a1B2c3"), Some(vec![0xa1, 0xb2, 0xc3]));
        assert_eq!(hex_decode("abc"), None);
        assert_eq!(hex_decode("zz"), None);
        assert_eq!(hex(&[0, 255]), "00ff");
    }
}

/// Test helpers shared by the modules' tests.
#[cfg(test)]
pub(crate) mod testutil {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::TempDir;

    pub(crate) fn tempdir() -> TempDir {
        match TempDir::new_in(&std::env::temp_dir(), "rg-assets-test-") {
            Ok(t) => t,
            Err(e) => panic!("temp dir: {e}"),
        }
    }

    /// A plain-HTTP loopback server (downloads accept http:// in this crate's tests only):
    /// path -> body, 404 otherwise. Returns the base URL and a request counter.
    pub(crate) fn serve(files: HashMap<String, Vec<u8>>) -> (String, Arc<AtomicUsize>) {
        let l = match TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(e) => panic!("bind: {e}"),
        };
        let base = format!(
            "http://{}",
            l.local_addr().map(|a| a.to_string()).unwrap_or_default()
        );
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                h.fetch_add(1, Ordering::SeqCst);
                let mut req = Vec::new();
                let mut buf = [0; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.extend_from_slice(&buf[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&req);
                let path = line.split(' ').nth(1).unwrap_or("");
                let (status, body) = match files.get(path) {
                    Some(b) => ("200 OK", b.as_slice()),
                    None => ("404 Not Found", &b"not found"[..]),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s
                    .write_all(head.as_bytes())
                    .and_then(|()| s.write_all(body));
            }
        });
        (base, hits)
    }

    /// A zip archive; a name ending in "@" is a symlink. Each file holds its own name.
    pub(crate) fn zip_of(names: &[&str]) -> Vec<u8> {
        use zip::write::SimpleFileOptions;
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for n in names {
            let res = match n.strip_suffix('@') {
                Some(link) => z.add_symlink(link, "target", opts),
                None => z
                    .start_file(*n, opts)
                    .and_then(|()| Ok(z.write_all(n.as_bytes())?)),
            };
            if let Err(e) = res {
                panic!("zip: {e}");
            }
        }
        match z.finish() {
            Ok(c) => c.into_inner(),
            Err(e) => panic!("zip: {e}"),
        }
    }

    /// `<n>.kext/Contents/Info.plist` and `<n>.kext/Contents/MacOS/<n>` for each name.
    pub(crate) fn kext_files(names: &[&str]) -> Vec<String> {
        names
            .iter()
            .flat_map(|n| {
                [
                    format!("{n}.kext/Contents/Info.plist"),
                    format!("{n}.kext/Contents/MacOS/{n}"),
                ]
            })
            .collect()
    }

    pub(crate) fn zip_kexts(names: &[&str], extra: &[&str]) -> Vec<u8> {
        let files = kext_files(names);
        let mut all: Vec<&str> = files.iter().map(String::as_str).collect();
        all.extend_from_slice(extra);
        zip_of(&all)
    }
}

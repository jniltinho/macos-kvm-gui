//! Installing a prebuilt package: download with a size cap, SHA-256 check, safe unpack.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Seek};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use rg_core::http::{Client, ClientOpts, HttpError};
use sha2::{Digest, Sha256};
use tar::EntryType;

use crate::{Error, Log, MAX_ENTRIES, MAX_PACKAGE, Reims, Result, activate, is_sha256, msg};

pub(crate) fn install(r: &Reims, cancel: &AtomicBool, log: Log<'_>) -> Result<()> {
    let rel = r.host_ubuntu_release().map_err(|e| {
        msg(format!(
            "{e}: there is no package for it (`reims-gui reims build --release 24.04|26.04` builds one in Docker)"
        ))
    })?;
    let m = r.manifest()?;
    let pkg = match m.packages.get(&rel) {
        Some(p) if is_sha256(&p.sha256) => p,
        _ => {
            return Err(msg(format!(
                "the app has no Reims vGPU package for Ubuntu {rel}: build it with `reims-gui reims build`"
            )));
        }
    };
    let (qdir, _lock) = r.lock(cancel)?;
    let name = format!("{}-ubuntu{rel}", m.tag);
    let final_dir = qdir.join(&name);
    if fs::metadata(&final_dir).is_ok() {
        log(&format!("{name} is already unpacked"));
    } else {
        let url = format!("{}{}/{}", r.release_url, m.tag, pkg.file);
        fetch(r, &url, &pkg.sha256, &final_dir, cancel, log)?;
    }
    activate::activate(r, &qdir, &name, &rel, log)
}

/// Downloads url, checks its SHA-256 and unpacks it into `final_dir`, which only appears once
/// complete.
fn fetch(
    r: &Reims,
    url: &str,
    want: &str,
    final_dir: &Path,
    cancel: &AtomicBool,
    log: Log<'_>,
) -> Result<()> {
    let parent = final_dir.parent().ok_or_else(|| msg("no parent folder"))?;
    let tmp = parent.join(format!(".tmp-{}", std::process::id()));
    fs::create_dir(&tmp)?;
    let res = (|| {
        let file = tmp.join("package");
        log(&format!("Downloading {url}"));
        download_to(r, url, &file, MAX_PACKAGE, cancel)?;
        let mut f = fs::File::open(&file)?;
        let mut h = Sha256::new();
        let mut buf = vec![0; 1 << 16];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        let got = hex::encode(h.finalize());
        if got != want {
            return Err(msg(format!(
                "{url}: the download's SHA-256 is {got}, the app expects {want}"
            )));
        }
        f.rewind()?;
        log(&format!("Unpacking into {}", final_dir.display()));
        let build = tmp.join("build");
        unpack(f, &build, r.max_expanded).map_err(|e| msg(format!("{url}: {e}")))?;
        fs::rename(&build, final_dir)?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&tmp);
    res
}

/// Saves an HTTPS URL into file, failing past max bytes. There is no overall timeout, as a
/// package takes a while on a slow line; redirects must stay on HTTPS.
pub(crate) fn download_to(
    r: &Reims,
    url: &str,
    file: &Path,
    max: u64,
    cancel: &AtomicBool,
) -> Result<()> {
    let client = Client::new(&ClientOpts {
        download: true,
        roots: r.roots.clone().unwrap_or_default(),
        ..ClientOpts::default()
    });
    let mut out = fs::File::create(file)?;
    client
        .get_to(url, max, &mut out, cancel)
        .map_err(|e| match e {
            HttpError::Cancelled => Error::Cancelled,
            HttpError::Failed(s) => msg(s),
        })?;
    out.sync_all()?;
    Ok(())
}

/// Fails once more than `left` bytes have been read through it.
struct Budget<R> {
    r: R,
    left: u64,
    max: u64,
}

impl<R: Read> Read for Budget<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.r.read(buf)?;
        self.left = self
            .left
            .checked_sub(n as u64)
            .ok_or_else(|| too_big(self.max))?;
        Ok(n)
    }
}

fn too_big(max: u64) -> io::Error {
    io::Error::other(format!(
        "the archive expands to more than {} MiB",
        max >> 20
    ))
}

/// Extracts a .tar.gz into dst, refusing absolute paths, `..`, links that are absolute or
/// contain `..`, anything written through a link, and entries other than files, folders and
/// links. Every byte decompressed counts against `max`, and so do the declared file sizes
/// (sparse entries expand past what the stream holds).
pub(crate) fn unpack(r: impl Read, dst: &Path, max: u64) -> Result<()> {
    let gz = flate2::read::MultiGzDecoder::new(r);
    let mut ar = tar::Archive::new(Budget {
        r: gz,
        left: max,
        max,
    });
    fs::create_dir_all(dst)?;
    let mut links = HashSet::new();
    let mut total = 0u64;
    for (n, entry) in ar.entries()?.enumerate() {
        let mut entry = entry?;
        if n >= MAX_ENTRIES {
            return Err(msg(format!(
                "the archive has more than {MAX_ENTRIES} entries"
            )));
        }
        let raw = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        if raw.starts_with('/') || raw.split('/').any(|c| c == "..") {
            return Err(msg(format!("unexpected path {raw:?} in the archive")));
        }
        let typ = entry.header().entry_type();
        let name = raw
            .split('/')
            .filter(|c| !c.is_empty() && *c != ".")
            .collect::<Vec<_>>()
            .join("/");
        if name.is_empty() {
            if typ != EntryType::Directory {
                return Err(msg(format!("unexpected entry {raw:?} in the archive")));
            }
            continue;
        }
        let mut parent = name.as_str();
        while let Some((p, _)) = parent.rsplit_once('/') {
            if links.contains(p) {
                return Err(msg(format!("{raw:?} is inside the link {p:?}")));
            }
            parent = p;
        }
        let target = dst.join(&name);
        if let Some(p) = target.parent() {
            fs::create_dir_all(p)?;
        }
        match typ {
            EntryType::Directory => fs::create_dir_all(&target)?,
            EntryType::Regular => {
                total = total.saturating_add(entry.size());
                if total > max {
                    return Err(too_big(max).into());
                }
                let mode = entry.header().mode()? & 0o777;
                // create_new never follows an existing link.
                let mut f = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(mode)
                    .open(&target)?;
                io::copy(&mut entry, &mut f)?;
            }
            EntryType::Symlink => {
                // Without ".." or a leading "/", a link (and any chain of them) only reaches
                // below its own folder; the packages have no other links.
                let link = entry
                    .link_name_bytes()
                    .map(|b| String::from_utf8_lossy(&b).into_owned());
                let link = link.unwrap_or_default();
                if link.starts_with('/') || link.split('/').any(|c| c == "..") {
                    return Err(msg(format!(
                        "the link {raw:?} points outside the archive ({link})"
                    )));
                }
                std::os::unix::fs::symlink(&link, &target)?;
                links.insert(name);
            }
            other => {
                return Err(msg(format!(
                    "unexpected entry {raw:?} in the archive (type {})",
                    other.as_byte() as char
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use std::collections::HashMap;

    fn reg(name: &str, body: &str) -> Entry {
        Entry::new(name, EntryType::Regular, "", body)
    }
    fn link(name: &str, to: &str) -> Entry {
        Entry::new(name, EntryType::Symlink, to, "")
    }

    #[test]
    fn unpack_extracts_files_and_inside_links() -> TestResult {
        let good = tar_gz(&[
            Entry::new("./bin/", EntryType::Directory, "", ""),
            reg("./bin/qemu", "x"),
            reg("./share/qemu/a.rom", "rom"),
            link("./share/qemu/b.rom", "a.rom"),
            link("./share/top", "qemu/a.rom"),
        ])?;
        let t = TempDir::new()?;
        let dst = t.path().join("b");
        unpack(good.as_slice(), &dst, crate::MAX_EXPANDED)?;
        assert_eq!(fs::read_to_string(dst.join("share/qemu/b.rom"))?, "rom");
        assert_eq!(fs::read_to_string(dst.join("share/top"))?, "rom");
        Ok(())
    }

    #[test]
    fn unpack_refuses_unsafe_archives() -> TestResult {
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("..", tar_gz(&[reg("bin/../../x", "")])?),
            ("absolute", tar_gz(&[reg("/etc/x", "")])?),
            ("outside link", tar_gz(&[link("bin/l", "../../x")])?),
            (
                "link chain",
                tar_gz(&[link("a", "."), link("escape", "a/..")])?,
            ),
            ("inside .. link", tar_gz(&[link("bin/l", "../share")])?),
            (". as a file", tar_gz(&[reg(".", "payload")])?),
            ("absolute link", tar_gz(&[link("l", "/etc/passwd")])?),
            ("through a link", tar_gz(&[link("d", "."), reg("d/x", "")])?),
            (
                "hard link",
                tar_gz(&[Entry::new("h", EntryType::Link, "bin/qemu", "")])?,
            ),
            (
                "device",
                tar_gz(&[Entry::new("c", EntryType::Char, "", "")])?,
            ),
            ("not a tar.gz", b"plain".to_vec()),
            ("a file over link", tar_gz(&[link("l", "x"), reg("l", "")])?),
        ];
        for (why, archive) in cases {
            let t = TempDir::new()?;
            let dst = t.path().join("sub").join("b");
            assert!(
                unpack(archive.as_slice(), &dst, crate::MAX_EXPANDED).is_err(),
                "{why}: accepted"
            );
            assert!(
                fs::symlink_metadata(t.path().join("x")).is_err(),
                "{why}: wrote outside"
            );
            assert!(
                fs::symlink_metadata(t.path().join("sub/x")).is_err(),
                "{why}: wrote outside"
            );
        }
        Ok(())
    }

    #[test]
    fn unpack_enforces_the_expansion_budget() -> TestResult {
        let big = "x".repeat(8192);
        let cases = [
            ("a file", tar_gz(&[reg("f", &big)])?),
            (
                "many files",
                tar_gz(&[reg("a", &big[..3000]), reg("b", &big[..3000])])?,
            ),
        ];
        for (why, archive) in cases {
            let t = TempDir::new()?;
            let err = unpack(archive.as_slice(), &t.path().join("b"), 4096)
                .err()
                .ok_or(why)?;
            assert!(err.to_string().contains("expands"), "{why}: {err}");
        }
        Ok(())
    }

    #[test]
    fn download_to_caps_the_size_and_needs_https() -> TestResult {
        let t = TempDir::new()?;
        let files = HashMap::from([
            ("/big".to_string(), vec![0; 11]),
            ("/ok".to_string(), vec![0; 10]),
        ]);
        let srv = https_server(files)?;
        let mut r = reims_in(&t, "")?;
        r.roots = Some(vec![srv.root.clone()]);
        let no = AtomicBool::new(false);
        let file = t.path().join("f");
        let err = download_to(&r, &format!("{}/big", srv.url), &file, 10, &no)
            .err()
            .ok_or("past the limit")?;
        assert!(err.to_string().contains("larger"), "{err}");
        download_to(&r, &format!("{}/ok", srv.url), &file, 10, &no)?;
        assert_eq!(fs::read(&file)?.len(), 10);
        let err = download_to(&r, &format!("{}/none", srv.url), &file, 10, &no)
            .err()
            .ok_or("404")?;
        assert!(err.to_string().contains("404"), "{err}");
        assert!(download_to(&r, "http://example.com/x", &file, 10, &no).is_err());
        Ok(())
    }

    #[test]
    fn install_checks_release_and_sha256_then_activates() -> TestResult {
        let _procs = QEMU_PROCS.lock();
        let t = TempDir::new()?;
        let pkg = fake_package()?;
        let srv = https_server(HashMap::from([(
            "/reims-test/good.tar.gz".to_string(),
            pkg.clone(),
        )]))?;
        let mut r = reims_in(
            &t,
            "ID=debian\nVERSION_ID=\"13\"\nPRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\n",
        )?;
        r.roots = Some(vec![srv.root.clone()]);
        r.release_url = format!("{}/", srv.url);
        r.ldd = "true".into();
        let manifest = |sum: &str| {
            format!(
                r#"{{"tag":"reims-test","commit":"c099dd5a4d61107a5527efc64cf1d1ab2f3a9b85","packages":{{"24.04":{{"file":"good.tar.gz","sha256":"{sum}"}}}}}}"#
            )
        };
        let no = AtomicBool::new(false);
        let log = |_: &str| {};

        let err = r
            .install(&no, &log)
            .err()
            .ok_or("installed on Debian")?
            .to_string();
        assert!(
            err.contains("24.04 and 26.04") && err.contains("reims build"),
            "{err}"
        );

        write(&r.os_release, "ID=ubuntu\nVERSION_ID=\"24.04\"\n")?;
        r.manifest_json = manifest(&"ab".repeat(32));
        let err = r
            .install(&no, &log)
            .err()
            .ok_or("a tampered package installed")?
            .to_string();
        assert!(err.contains("SHA-256"), "{err}");
        assert_eq!(
            fs::read_dir(r.dir.join("qemu"))?.count(),
            0,
            "nothing is unpacked"
        );

        r.manifest_json = manifest(&hex::encode(Sha256::digest(&pkg)));
        r.install(&no, &log)?;
        assert_eq!(
            fs::read_link(r.dir.join("qemu/current"))?,
            Path::new("reims-test-ubuntu24.04")
        );
        let info = r.installed()?;
        assert_eq!(
            (info.build.as_str(), info.commit.as_str()),
            ("reims-test-ubuntu24.04", "test")
        );
        assert!(info.apple_gfx.is_ok(), "{:?}", info.apple_gfx);
        Ok(())
    }

    /// Installs a real package for this host, as the manifest pins it, from a local HTTPS server:
    /// `MKG_REIMS_PACKAGES=<folder with the release's files> cargo test -p rg-reims real_package`.
    #[test]
    fn install_real_package_when_given() -> TestResult {
        let Some(dir) = std::env::var_os("MKG_REIMS_PACKAGES") else {
            return Ok(());
        };
        let _procs = QEMU_PROCS.lock();
        let t = TempDir::new()?;
        let mut r = reims_in(&t, &fs::read_to_string("/etc/os-release")?)?;
        let Ok(rel) = r.host_ubuntu_release() else {
            return Ok(());
        };
        let m = crate::manifest()?;
        let file = &m.packages.get(&rel).ok_or("no package")?.file;
        let data = fs::read(Path::new(&dir).join(file))?;
        let srv = https_server(HashMap::from([(format!("/{}/{file}", m.tag), data)]))?;
        r.roots = Some(vec![srv.root.clone()]);
        r.release_url = format!("{}/", srv.url);
        r.install(&AtomicBool::new(false), &|l| println!("{l}"))?;
        let info = r.installed()?;
        assert!(info.apple_gfx.is_ok(), "{:?}", info.apple_gfx);
        assert_eq!(info.build, format!("{}-ubuntu{rel}", m.tag));
        Ok(())
    }
}

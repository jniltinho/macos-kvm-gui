//! Extra kexts: OpenCore kexts the user lists in settings.yaml, for example a patch kext for an
//! AMD GPU macOS does not drive natively. The app never names, pins or fetches a specific kext:
//! every URL comes from the user (Go `extrakexts.go`).

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::boot::{KextCache, copy_kext};
use crate::compat::{ExtraKext, ExtraKexts};
use crate::http::https_get_max;
use crate::rootpatch::{root_patch_kexts, unpack_kext};
use crate::util::{TempDir, sha256_hex};
use crate::{Context, Error, Result, invalid};

// Archive limits: a size over a limit fails the download, nothing is ever truncated.
const MAX_KEXT_ARCHIVE: u64 = 32 << 20;
const MAX_KEXT_EXPANDED: u64 = 128 << 20;
/// unpack_kext's per-file limit.
pub(crate) const MAX_KEXT_FILE: u64 = 16 << 20;
const MAX_KEXT_ENTRIES: usize = 4096;

/// Valid on FAT, nothing to escape in XML: `^[A-Za-z0-9._+-]{1,64}$`.
pub(crate) fn valid_kext_name(n: &str) -> bool {
    (1..=64).contains(&n.len())
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

/// `^[0-9a-f]{64}$`.
pub(crate) fn valid_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl ExtraKext {
    /// Identifies the entry's cache folder: any change to the entry is another folder.
    fn id(&self) -> String {
        format!(
            "{} sha256={} load={}",
            self.url,
            self.sha256,
            self.kexts.as_deref().unwrap_or_default().join(":")
        )
    }

    fn dir(&self, base: &Path) -> PathBuf {
        base.join(sha256_hex(self.id().as_bytes()))
    }

    /// The entry's kexts in load order. The folder's "entry" file holds the entry text, then
    /// one kext name per line; a folder whose text differs is not this entry's.
    fn cached(&self, base: &Path) -> Option<Vec<String>> {
        let dir = self.dir(base);
        let text = fs::read_to_string(dir.join("entry")).ok()?;
        let lines: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
        if lines[0] != self.id() || lines.len() < 2 {
            return None;
        }
        for n in &lines[1..] {
            let kext = dir.join(format!("{n}.kext/Contents"));
            for f in [kext.join("MacOS").join(n), kext.join("Info.plist")] {
                if !fs::symlink_metadata(f).is_ok_and(|m| m.is_file()) {
                    return None;
                }
            }
        }
        Some(lines[1..].iter().map(|s| s.to_string()).collect())
    }

    fn download(&self, base: &Path, cancel: &AtomicBool) -> Result<()> {
        let data = https_get_max(&self.url, MAX_KEXT_ARCHIVE, cancel)?;
        if !self.sha256.is_empty() {
            let sum = sha256_hex(&data);
            if sum != self.sha256 {
                return invalid(format!(
                    "the download's SHA-256 is {sum}, the entry expects {}",
                    self.sha256
                ));
            }
        }
        let names = check_kext_archive(&data, self.kexts.as_deref())?;
        let tmp = TempDir::new_in(base, ".tmp-")?;
        for n in &names {
            unpack_kext(&data, n, tmp.path()).context(|| format!("{n}.kext"))?;
        }
        let entry = format!("{}\n{}\n", self.id(), names.join("\n"));
        fs::write(tmp.path().join("entry"), entry)?;
        let final_ = self.dir(base);
        let _ = fs::remove_dir_all(&final_); // not a valid copy of this entry, or cached() would have taken it
        fs::rename(tmp.path(), final_)?;
        Ok(())
    }
}

/// Go `ExtraKexts.Ready`: every entry is cached, so a start needs no network.
pub fn extra_kexts_ready(ks: &ExtraKexts, cache: &KextCache) -> bool {
    let base = cache.extra_dir();
    ks.entries.iter().all(|e| e.cached(&base).is_some())
}

/// Go `DownloadExtraKexts`: fetches the entries that are not cached yet. A host-wide lock on
/// the cache makes concurrent starts download an entry once; each entry is published by
/// renaming a complete temporary folder. Waiting for the lock and downloading stop when
/// `cancel` is set.
pub fn download_extra_kexts(ks: &ExtraKexts, cache: &KextCache, cancel: &AtomicBool) -> Result<()> {
    if ks.entries.is_empty() {
        return Ok(());
    }
    let base = cache.extra_dir();
    fs::create_dir_all(&base)?;
    let _lock = lock_file(&base.join(".lock"), cancel)?;
    // Leftovers of a start that died while it held the lock.
    if let Ok(rd) = fs::read_dir(&base) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(".tmp-") {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }
    for e in &ks.entries {
        if e.cached(&base).is_none() {
            e.download(&base, cancel).context(|| e.url.clone())?;
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    Ok(())
}

/// A host-wide exclusive flock on `path`, polling for another holder until `cancel` is set.
/// The lock is released when the returned file is dropped.
fn lock_file(path: &Path, cancel: &AtomicBool) -> Result<File> {
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    loop {
        match f.try_lock() {
            Ok(()) => return Ok(f),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Go `checkKextArchive`: validates the whole archive before anything is extracted and returns
/// the kexts to load, in order: the archive's only root-level kext, or the ones `want` lists.
pub(crate) fn check_kext_archive(data: &[u8], want: Option<&[String]>) -> Result<Vec<String>> {
    let mut zr = zip::ZipArchive::new(Cursor::new(data))?;
    if zr.len() > MAX_KEXT_ENTRIES {
        return invalid(format!(
            "the archive has more than {MAX_KEXT_ENTRIES} entries"
        ));
    }
    let mut total = 0u64;
    let mut regular = std::collections::HashSet::new();
    let mut found = Vec::new();
    for i in 0..zr.len() {
        let f = zr.by_index_raw(i)?;
        let name = f.name().to_string();
        if name.starts_with('/') || name.split('/').any(|p| p == "..") {
            return invalid(format!("unexpected path {name:?} in the archive"));
        }
        if f.is_symlink() {
            return invalid(format!("unexpected link {name:?} in the archive"));
        }
        if f.size() > MAX_KEXT_FILE {
            return invalid(format!("{name} is larger than {} MiB", MAX_KEXT_FILE >> 20));
        }
        total += f.size();
        if total > MAX_KEXT_EXPANDED {
            return invalid(format!(
                "the archive expands to more than {} MiB",
                MAX_KEXT_EXPANDED >> 20
            ));
        }
        // Regular: not a folder, and no Unix file type other than a regular file.
        if f.is_dir()
            || f.unix_mode()
                .is_some_and(|m| m & 0o170000 != 0 && m & 0o170000 != 0o100000)
        {
            continue;
        }
        let p: Vec<&str> = name.split('/').collect();
        if p.len() == 4 && p[1] == "Contents" && p[2] == "MacOS" && p[0] == format!("{}.kext", p[3])
        {
            found.push(p[3].to_string());
        }
        regular.insert(name);
    }
    let want: Vec<String> = match want {
        Some(w) => w.to_vec(),
        None => match found.len() {
            0 => return invalid("the archive has no root-level <Name>.kext/Contents/MacOS/<Name>"),
            1 => found.clone(),
            _ => {
                let l = found.join(", ");
                return invalid(format!(
                    "the archive has several kexts ({l}): choose them, in load order, with load: [{l}]"
                ));
            }
        },
    };
    for n in &want {
        if !valid_kext_name(n) {
            return invalid(format!("unsupported kext name {n:?}"));
        }
        if !found.contains(n) {
            return invalid(format!("the archive has no {n}.kext/Contents/MacOS/{n}"));
        }
        if !regular.contains(&format!("{n}.kext/Contents/Info.plist")) {
            return invalid(format!("{n}.kext has no Contents/Info.plist"));
        }
    }
    Ok(want)
}

/// The snapshot's cached kexts in load order, as (folder, name); refuses names that clash,
/// ignoring case (OpenCore's disk is FAT), with each other or with `taken`.
fn names(ks: &ExtraKexts, base: &Path, taken: &[String]) -> Result<Vec<(PathBuf, String)>> {
    let mut seen: HashMap<String, String> = taken
        .iter()
        .map(|n| (n.to_lowercase(), format!("{n}.kext")))
        .collect();
    let mut out = Vec::new();
    for e in &ks.entries {
        let Some(names) = e.cached(base) else {
            return invalid(format!(
                "the settings.yaml entry {} is not downloaded: start the VM from the app, `vm start` or the MCP server, which download it",
                e.url
            ));
        };
        for n in names {
            if let Some(other) = seen.get(&n.to_lowercase()) {
                return invalid(format!("{n}.kext from settings.yaml clashes with {other}"));
            }
            seen.insert(n.to_lowercase(), format!("{n}.kext from {}", e.url));
            out.push((e.dir(base), n));
        }
    }
    Ok(out)
}

/// Go `kextSwitches`: renders the template's `@@ENABLED_<Name>@@` switches: `<false/>` for
/// the built-in kexts settings.yaml disables, `<true/>` for the others.
pub fn kext_switches(tmpl: &str, disabled: &[String]) -> Result<String> {
    for n in disabled {
        if n.eq_ignore_ascii_case("Lilu") {
            return invalid("settings.yaml disable: Lilu is required by every other kext");
        }
        if !tmpl.contains(&format!("@@ENABLED_{n}@@")) {
            return invalid(format!(
                "settings.yaml disable: {n} is not a built-in kext the config template in use can disable"
            ));
        }
    }
    // `@@ENABLED_([A-Za-z0-9]+)@@`; two such markers can never overlap.
    const MARK: &str = "@@ENABLED_";
    let mut out = String::with_capacity(tmpl.len());
    let mut rest = tmpl;
    while let Some(i) = rest.find(MARK) {
        out.push_str(&rest[..i]);
        let after = &rest[i + MARK.len()..];
        let n = after.bytes().take_while(u8::is_ascii_alphanumeric).count();
        if n > 0 && after[n..].starts_with("@@") {
            let on = !disabled.iter().any(|d| d == &after[..n]);
            out.push_str(if on { "<true/>" } else { "<false/>" });
            rest = &after[n + 2..];
        } else {
            out.push_str(MARK);
            rest = after;
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// Go `addExtraKexts`: copies the snapshot's kexts into the extracted EFI, after the built-in,
/// root-patch and ReimsVgpu kexts, and returns their names for the config.
pub(crate) fn add_extra_kexts(
    ks: &ExtraKexts,
    cache: &KextCache,
    builtin: &[String],
    kexts_dir: &Path,
) -> Result<Vec<String>> {
    let mut taken = builtin.to_vec();
    taken.push("ReimsVgpu".into());
    taken.extend(root_patch_kexts().iter().map(|k| k.name.to_string()));
    let kexts = names(ks, &cache.extra_dir(), &taken)?;
    for (dir, n) in &kexts {
        copy_kext(&dir.join(format!("{n}.kext")), kexts_dir, n)?;
    }
    Ok(kexts.into_iter().map(|(_, n)| n).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BootAssets;
    use crate::boot::{BootOptions, BootState, EFI_DIR, Identity, opencore_config, prepare_boot};
    use crate::util::exists;
    use crate::util::testutil::{kext_files, serve, tempdir, zip_kexts, zip_of};
    use rg_core::Config;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    fn err_text<T>(r: Result<T>) -> String {
        r.err().map(|e| e.to_string()).unwrap_or_default()
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn check_kext_archive_accepts_and_orders() -> Result<()> {
        assert_eq!(
            check_kext_archive(&zip_kexts(&["Foo"], &[]), None)?,
            ["Foo"]
        );
        assert_eq!(
            check_kext_archive(&zip_kexts(&["A", "B"], &[]), Some(&strings(&["B", "A"])))?,
            ["B", "A"]
        );
        assert!(
            check_kext_archive(&zip_kexts(&["A"], &[]), Some(&strings(&["B"]))).is_err(),
            "load naming a missing kext"
        );
        Ok(())
    }

    #[test]
    fn check_kext_archive_refuses_bad_archives() -> Result<()> {
        let big = {
            use std::io::Write;
            let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let o = zip::write::SimpleFileOptions::default();
            for n in kext_files(&["Big"]) {
                z.start_file(n.as_str(), o)?;
                if n.contains("MacOS") {
                    z.write_all(&vec![0; MAX_KEXT_FILE as usize + 1])?;
                }
            }
            z.finish()?.into_inner()
        };
        for (name, data, want) in [
            ("outside link", zip_kexts(&["Foo"], &["elsewhere@"]), "link"),
            ("dot-dot", zip_kexts(&["Foo"], &["x/../../y"]), "path"),
            ("absolute", zip_kexts(&["Foo"], &["/etc/x"]), "path"),
            (
                "no Info.plist",
                zip_of(&["Foo.kext/Contents/MacOS/Foo"]),
                "Info.plist",
            ),
            ("no kext", zip_of(&["README"]), "no root-level"),
            ("several", zip_kexts(&["A", "B"], &[]), "load: [A, B]"),
            (
                "nested",
                zip_of(&[
                    "dir/Foo.kext/Contents/Info.plist",
                    "dir/Foo.kext/Contents/MacOS/Foo",
                ]),
                "no root-level",
            ),
            ("file over limit", big, "larger than"),
        ] {
            let e = err_text(check_kext_archive(&data, None));
            assert!(e.contains(want), "{name}: want {want:?}, got {e:?}");
        }
        Ok(())
    }

    fn server(files: &[(&str, Vec<u8>)]) -> (String, Arc<AtomicUsize>) {
        serve(
            files
                .iter()
                .map(|(p, d)| (p.to_string(), d.clone()))
                .collect(),
        )
    }

    fn entry(url: String, sha256: &str, load: Option<&[&str]>) -> ExtraKext {
        ExtraKext {
            url,
            sha256: sha256.into(),
            kexts: load.map(strings),
        }
    }

    #[test]
    fn download_checks_hash_and_downloads_once() -> Result<()> {
        let root = tempdir();
        let cache = KextCache::new(root.path());
        let foo = zip_kexts(&["Foo"], &[]);
        let (srv, hits) = server(&[("/foo.zip", foo.clone())]);
        let no = AtomicBool::new(false);
        let sum = sha256_hex(&foo);
        let bad = ExtraKexts {
            entries: vec![entry(format!("{srv}/foo.zip"), &"0".repeat(64), None)],
            ..Default::default()
        };
        assert!(
            err_text(download_extra_kexts(&bad, &cache, &no)).contains(&sum),
            "a hash mismatch names both hashes"
        );
        let left: Vec<_> = fs::read_dir(cache.extra_dir())?
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, [".lock"], "a mismatch caches nothing");

        let ks = ExtraKexts {
            entries: vec![entry(format!("{srv}/foo.zip"), &sum, None)],
            ..Default::default()
        };
        hits.store(0, Ordering::SeqCst);
        std::thread::scope(|s| {
            let a = s.spawn(|| download_extra_kexts(&ks, &cache, &no));
            let b = s.spawn(|| download_extra_kexts(&ks, &cache, &no));
            for h in [a, b] {
                assert!(matches!(h.join(), Ok(Ok(()))));
            }
        });
        assert_eq!(hits.load(Ordering::SeqCst), 1, "two starts download once");
        assert!(extra_kexts_ready(&ks, &cache));

        // A folder whose entry file names another entry is not a cache hit.
        fs::write(
            ks.entries[0].dir(&cache.extra_dir()).join("entry"),
            "https://other\nFoo\n",
        )?;
        assert!(
            !extra_kexts_ready(&ks, &cache),
            "a forged folder does not count"
        );
        download_extra_kexts(&ks, &cache, &no)?;
        assert!(extra_kexts_ready(&ks, &cache) && hits.load(Ordering::SeqCst) == 2);
        Ok(())
    }

    #[test]
    fn cache_edges_and_cancelled_lock_wait() -> Result<()> {
        let mut e = entry("https://example.com/a.zip".into(), "", Some(&[]));
        e.check()?;
        assert_eq!(
            e.id(),
            entry("https://example.com/a.zip".into(), "", None).id(),
            "load: [] is no load list"
        );

        let root = tempdir();
        let cache = KextCache::new(root.path());
        let (srv, _) = server(&[("/foo.zip", zip_kexts(&["Foo"], &[]))]);
        let ks = ExtraKexts {
            entries: vec![entry(format!("{srv}/foo.zip"), "", None)],
            ..Default::default()
        };
        let no = AtomicBool::new(false);
        download_extra_kexts(&ks, &cache, &no)?;
        assert!(extra_kexts_ready(&ks, &cache));
        fs::remove_file(
            ks.entries[0]
                .dir(&cache.extra_dir())
                .join("Foo.kext/Contents/Info.plist"),
        )?;
        assert!(
            !extra_kexts_ready(&ks, &cache),
            "a cached kext without Info.plist is not a hit"
        );

        // A start waiting for another one's download gives up when cancelled.
        let held = File::open(cache.extra_dir().join(".lock"))?;
        held.lock()?;
        let cancel = AtomicBool::new(false);
        let r = std::thread::scope(|s| {
            let h = s.spawn(|| download_extra_kexts(&ks, &cache, &cancel));
            std::thread::sleep(Duration::from_millis(300));
            cancel.store(true, Ordering::Relaxed);
            h.join()
        });
        assert!(matches!(r, Ok(Err(Error::Cancelled))));
        Ok(())
    }

    #[test]
    fn extra_kexts_boot_order_removal_and_clashes() -> Result<()> {
        let root = tempdir();
        let cache = KextCache::new(root.path());
        let (srv, _) = server(&[
            ("/foo.zip", zip_kexts(&["Foo"], &[])),
            ("/pair.zip", zip_kexts(&["Plugin", "Provider"], &[])),
            ("/weg.zip", zip_kexts(&["Whatevergreen"], &[])),
            ("/rv.zip", zip_kexts(&["ReimsVgpu"], &[])),
        ]);
        let foo = entry(format!("{srv}/foo.zip"), "", None);
        let pair = entry(format!("{srv}/pair.zip"), "", Some(&["Provider", "Plugin"]));
        let ws = root.path().join("vm");
        let mut c = Config::defaults();
        let a = BootAssets::Embedded;
        let no = AtomicBool::new(false);
        let read =
            || fs::read_to_string(ws.join(EFI_DIR).join("EFI/OC/config.plist")).unwrap_or_default();
        fn opts(x: Option<&ExtraKexts>) -> BootOptions<'_> {
            BootOptions {
                extras: x,
                keyboard: "en-US:0",
                ..Default::default()
            }
        }

        prepare_boot(&ws, &c, &a, &cache, &opts(None))?;
        let before = read();
        let empty = ExtraKexts::default();
        prepare_boot(&ws, &c, &a, &cache, &opts(Some(&empty)))?;
        assert_eq!(read(), before, "an empty settings.yaml changes nothing");

        let ks = ExtraKexts {
            entries: vec![foo, pair],
            ..Default::default()
        };
        assert!(
            err_text(prepare_boot(&ws, &c, &a, &cache, &opts(Some(&ks))))
                .contains("not downloaded")
        );
        download_extra_kexts(&ks, &cache, &no)?;

        c.root_patch = true;
        c.version = "sequoia".into();
        for k in root_patch_kexts() {
            let d = cache
                .root_patch_dir()
                .join(format!("{0}.kext/Contents/MacOS", k.name));
            fs::create_dir_all(&d)?;
            fs::write(d.join(k.name), "")?;
            fs::write(d.join("../Info.plist"), "")?;
        }
        prepare_boot(&ws, &c, &a, &cache, &opts(Some(&ks)))?;
        let cfg = read();
        let mut last = 0;
        for k in [
            "Lilu",
            "VirtualSMC",
            "WhateverGreen",
            "AMFIPass",
            "KDKlessWorkaround",
            "Foo",
            "Provider",
            "Plugin",
        ] {
            let i = cfg.find(&format!("<string>{k}.kext</string>")).unwrap_or(0);
            assert!(i > last, "{k} is out of order");
            last = i;
        }
        assert!(
            exists(
                &ws.join(EFI_DIR)
                    .join("EFI/OC/Kexts/Plugin.kext/Contents/MacOS/Plugin")
            ) && !cfg.contains("@@")
        );

        c.root_patch = false;
        prepare_boot(&ws, &c, &a, &cache, &opts(Some(&empty)))?;
        assert!(
            !exists(&ws.join(EFI_DIR).join("EFI/OC/Kexts/Foo.kext"))
                && !read().contains("Foo.kext"),
            "a removed entry is gone"
        );

        let weg = ExtraKexts {
            entries: vec![entry(format!("{srv}/weg.zip"), "", None)],
            ..Default::default()
        };
        download_extra_kexts(&weg, &cache, &no)?;
        assert!(
            err_text(prepare_boot(&ws, &c, &a, &cache, &opts(Some(&weg))))
                .contains("WhateverGreen.kext"),
            "a clash ignoring case"
        );
        let rv = ExtraKexts {
            entries: vec![entry(format!("{srv}/rv.zip"), "", None)],
            ..Default::default()
        };
        download_extra_kexts(&rv, &cache, &no)?;
        assert!(
            err_text(prepare_boot(&ws, &c, &a, &cache, &opts(Some(&rv))))
                .contains("clashes with ReimsVgpu.kext")
        );
        Ok(())
    }

    #[test]
    fn kext_switches_disable_built_in_kexts() -> Result<()> {
        let id = Identity::generate()?;
        let c = Config::defaults();
        let a = BootAssets::Embedded;
        let mut ks = ExtraKexts {
            disabled: strings(&["WhateverGreen"]),
            ..Default::default()
        };
        let r = |ks: &ExtraKexts| {
            opencore_config(
                &a,
                &c,
                &id,
                &BootOptions {
                    extras: Some(ks),
                    keyboard: "en-US:0",
                    ..Default::default()
                },
                &BootState::default(),
            )
        };
        let cfg = r(&ks)?;
        let entry = |n: &str| {
            let i = cfg
                .find(&format!(
                    "<string>{n}.kext</string>\n\t\t\t\t<key>Comment</key>"
                ))
                .unwrap_or(0);
            cfg[i..i + 200].to_string()
        };
        assert!(
            entry("WhateverGreen").contains("<false/>")
                && entry("VirtualSMC").contains("<true/>")
                && entry("Lilu").contains("<true/>")
        );
        for n in ["Lilu", "Nope"] {
            ks.disabled = strings(&[n]);
            assert!(err_text(r(&ks)).contains(n), "{n}");
        }
        let tmpl = String::from_utf8_lossy(&a.read("config.plist.tmpl")?)
            .replace("@@ENABLED_VirtualSMC@@", "<true/>")
            .replace("@@ENABLED_WhateverGreen@@", "<true/>");
        assert!(tmpl.contains("<key>Enabled</key>\n\t\t\t\t<true/>"));
        assert!(
            kext_switches(&tmpl, &[]).is_ok(),
            "an override template without switches still works"
        );
        assert!(err_text(kext_switches(&tmpl, &strings(&["WhateverGreen"]))).contains("template"));
        Ok(())
    }

    #[test]
    fn name_checks() {
        assert!(
            valid_kext_name("Lilu")
                && valid_kext_name("a.b_c+d-1")
                && !valid_kext_name("a/b")
                && !valid_kext_name("")
        );
        assert!(!valid_kext_name(&"a".repeat(65)));
        assert!(
            valid_sha256(&"ab".repeat(32))
                && !valid_sha256(&"AB".repeat(32))
                && !valid_sha256("abc")
        );
    }
}

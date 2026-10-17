//! The full macOS installer: Apple's InstallAssistant.pkg (Big Sur and later, 12–18 GB), found in
//! the Software Update catalog, downloaded once into a cache shared by every VM
//! (`<Workspace>/.installers/<build>/`), checked against its integrity data, and kept as an
//! ISO 9660 disc the VM gets while it has install media, so the recovery installs with no network.

use std::cmp::Ordering;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use plist::Value;
use regex::bytes::Regex;
use serde::{Deserialize, Serialize};

use crate::chunklist::{parse_integrity, verify_chunks};
use crate::http::{Http, copy_body, http_err, status_text};
use crate::{Error, Progress, Result, check_cancel, other};

/// The macOS major version of each app version that has an InstallAssistant.
const INSTALLER_MAJOR: [(&str, u32); 6] = [
    ("big-sur", 11),
    ("monterey", 12),
    ("ventura", 13),
    ("sonoma", 14),
    ("sequoia", 15),
    ("tahoe", 26),
];

/// Board IDs of the Mac models the VMs present; a product whose distribution does not list it
/// is for other Macs (for example Apple silicon only).
const BOARD_IDS: [(&str, &str); 3] = [
    ("iMac20,1", "Mac-CFF7D910A743CAAF"),
    ("iMac19,1", "Mac-AA95B1DDAB278B95"),
    ("iMac18,3", "Mac-BE088AF8C5EB4FA2"),
];

fn installer_major(version: &str) -> Option<u32> {
    INSTALLER_MAJOR
        .iter()
        .find(|(v, _)| *v == version)
        .map(|(_, m)| *m)
}

/// The major number of a dotted version ("15.8.1" → 15), 0 if it has none.
fn major_of(version: &str) -> u32 {
    version
        .split('.')
        .next()
        .and_then(|m| m.parse().ok())
        .unwrap_or(0)
}

/// One InstallAssistant.pkg in the catalog; also the cache's `info.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InstallerProduct {
    pub product_id: String,
    pub version: String,
    pub build: String,
    pub url: String,
    pub size: u64,
    pub integrity_url: String,
}

/// Whether a macOS version has a full installer in the catalog (Big Sur and later).
pub fn installer_supported(version: &str) -> bool {
    installer_major(version).is_some()
}

/// Lists the catalog's installers for a macOS version that support the VM's Mac model, newest
/// first. Distributions are fetched six at a time.
pub fn find_installers(http: &Http, version: &str) -> Result<Vec<InstallerProduct>> {
    let Some(major) = installer_major(version) else {
        return other(format!(
            "Apple has no full installer package for {}: use the recovery",
            rg_core::config::version_display(version)
        ));
    };
    let mut last = Error::Other("no catalog".into());
    let mut cat = None;
    for u in &http.catalogs {
        match http.https_get(u, 64 << 20) {
            Ok(c) => {
                cat = Some(c);
                break;
            }
            Err(e) => last = e,
        }
    }
    let Some(mut cat) = cat else {
        return other(format!("Apple's software catalog: {last}"));
    };
    if cat.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(cat.as_slice())
            .take(256 << 20)
            .read_to_end(&mut out)?;
        cat = out;
    }
    let candidates = catalog_installers(&cat)?;
    let board = BOARD_IDS
        .iter()
        .find(|(m, _)| *m == rg_core::config::smbios_model(version))
        .map(|(_, b)| b.as_bytes());

    let next = AtomicUsize::new(0);
    let out = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..6 {
            s.spawn(|| {
                while let Some((c, dist_url)) =
                    candidates.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                {
                    let Ok(dist) = http.https_get(dist_url, 4 << 20) else {
                        continue;
                    };
                    let mut p = c.clone();
                    (p.version, p.build) = dist_info(&dist);
                    if major_of(&p.version) != major {
                        continue;
                    }
                    if board.is_some_and(|b| !dist.windows(b.len()).any(|w| w == b)) {
                        continue; // for other Macs, e.g. Apple silicon only
                    }
                    out.lock().unwrap_or_else(|e| e.into_inner()).push(p);
                }
            });
        }
    });
    let mut out = out.into_inner().unwrap_or_else(|e| e.into_inner());
    out.sort_by(|a, b| version_cmp(&b.version, &a.version));
    if out.is_empty() {
        return other(format!(
            "Apple's catalog has no full installer of {} for this Mac model",
            rg_core::config::version_display(version)
        ));
    }
    Ok(out)
}

/// The products that carry an InstallAssistant.pkg, with their English distribution's URL.
fn catalog_installers(catalog: &[u8]) -> Result<Vec<(InstallerProduct, String)>> {
    let root = Value::from_reader(io::Cursor::new(catalog))
        .map_err(|e| Error::Other(format!("Apple's software catalog: {e}")))?;
    let mut out = Vec::new();
    let Some(products) = root
        .as_dictionary()
        .and_then(|d| d.get("Products"))
        .and_then(Value::as_dictionary)
    else {
        return Ok(out);
    };
    for (id, p) in products {
        let Some(p) = p.as_dictionary() else { continue };
        let meta = p.get("ExtendedMetaInfo").and_then(Value::as_dictionary);
        if !meta.is_some_and(|m| m.contains_key("InstallAssistantPackageIdentifiers")) {
            continue;
        }
        let dists = p.get("Distributions").and_then(Value::as_dictionary);
        let dist = dists
            .and_then(|d| d.get("English").or_else(|| d.get("en")))
            .and_then(Value::as_string)
            .unwrap_or_default();
        for pk in p
            .get("Packages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(pk) = pk.as_dictionary() else {
                continue;
            };
            let url = pk.get("URL").and_then(Value::as_string).unwrap_or_default();
            if !url.ends_with("/InstallAssistant.pkg") || dist.is_empty() {
                continue;
            }
            let product = InstallerProduct {
                product_id: id.clone(),
                url: url.to_owned(),
                size: pk
                    .get("Size")
                    .and_then(Value::as_unsigned_integer)
                    .unwrap_or(0),
                integrity_url: pk
                    .get("IntegrityDataURL")
                    .and_then(Value::as_string)
                    .unwrap_or_default()
                    .to_owned(),
                ..InstallerProduct::default()
            };
            out.push((product, dist.to_owned()));
        }
    }
    Ok(out)
}

static DIST_VERSION: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"<key>(?:VERSION|macOSProductVersion)</key>\s*<string>([^<]+)</string>").ok()
});
static DIST_BUILD: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"<key>(?:BUILD|macOSProductBuildVersion)</key>\s*<string>([^<]+)</string>").ok()
});

/// The version and build in a distribution file.
fn dist_info(dist: &[u8]) -> (String, String) {
    let find = |re: &Option<Regex>| {
        re.as_ref()
            .and_then(|re| re.captures(dist))
            .and_then(|c| c.get(1))
            .map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned())
            .unwrap_or_default()
    };
    (find(&DIST_VERSION), find(&DIST_BUILD))
}

/// Compares dotted versions numerically (15.10 > 15.9); a part that is not a number counts 0.
fn version_cmp(a: &str, b: &str) -> Ordering {
    let (pa, pb): (Vec<&str>, Vec<&str>) = (a.split('.').collect(), b.split('.').collect());
    let num = |p: &[&str], i: usize| p.get(i).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    (0..pa.len().max(pb.len()))
        .map(|i| num(&pa, i).cmp(&num(&pb, i)))
        .find(|o| o.is_ne())
        .unwrap_or(Ordering::Equal)
}

// --- cache -------------------------------------------------------------------------------------

/// Only while downloading: the cache keeps the disc.
const INSTALLER_FILE: &str = "InstallAssistant.pkg";

/// A verified installer in the shared cache, as an installer disc.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedInstaller {
    pub product: InstallerProduct,
    /// The `.iso`.
    pub path: PathBuf,
}

/// The shared cache, `<workspace>/.installers`, for the resolved (absolute) Workspace folder.
pub fn installers_dir(workspace: &Path) -> PathBuf {
    workspace.join(".installers")
}

/// The verified installers in `dir` (see [`installers_dir`]), newest first.
pub fn cached_installers(dir: &Path) -> Vec<CachedInstaller> {
    let mut out: Vec<CachedInstaller> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let b = fs::read(e.path().join("info.json")).ok()?;
            let product: InstallerProduct = serde_json::from_slice(&b).ok()?;
            let path = e.path().join(DISC_FILE);
            (valid_build(&product.build) && fs::metadata(&path).ok()?.len() > product.size)
                .then_some(CachedInstaller { product, path })
        })
        .collect();
    out.sort_by(|a, b| version_cmp(&b.product.version, &a.product.version));
    out
}

/// The newest cached installer of a macOS version, if any.
pub fn cached_installer_for(dir: &Path, version: &str) -> Option<CachedInstaller> {
    let major = installer_major(version)?;
    cached_installers(dir)
        .into_iter()
        .find(|c| major_of(&c.product.version) == major)
}

/// An Apple build id (`24H32`, `25A5279m`), Go `buildRe`. It names a directory under the cache,
/// so it must never be `.`, `..` or contain a separator.
fn valid_build(s: &str) -> bool {
    static BUILD: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"^[0-9]{2}[A-Z][0-9]{1,5}[a-z]?$").ok());
    BUILD.as_ref().is_some_and(|re| re.is_match(s.as_bytes()))
}

/// Deletes a cached installer; one attached to a running VM is kept.
pub fn remove_installer(dir: &Path, build: &str) -> Result<()> {
    if !valid_build(build) {
        return other(format!("invalid build {build:?}"));
    }
    let path = dir.join(build);
    // The build is a single path element; refuse a symlink so the delete never leaves the cache.
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return other(format!(
            "{} is a symlink, not a cached installer",
            path.display()
        ));
    }
    if disc_in_use(&path.join(DISC_FILE)) {
        return other("a running VM is using this installer: shut it down first");
    }
    match fs::remove_dir_all(&path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Downloads (resuming a partial download), verifies and caches the newest full installer of
/// the macOS version into `dir` (see [`installers_dir`]), unless it is already cached. One
/// download per build across processes (the app, the CLI, the MCP server), through a flock.
pub fn download_installer(
    http: &Http,
    dir: &Path,
    version: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
    log: &mut dyn FnMut(&str),
) -> Result<CachedInstaller> {
    progress(Progress::new("Reading Apple's software catalog", 0, 0));
    let products = find_installers(http, version)?;
    let Some(p) = products.into_iter().next() else {
        return other("no installer");
    };
    if let Some(c) = cached_installer_for(dir, version)
        && c.product.build == p.build
    {
        return Ok(c);
    }
    log(&format!(
        "[installer] macOS {} ({}), {:.1} GB from {}",
        p.version,
        p.build,
        p.size as f64 / 1e9,
        p.url
    ));
    if !valid_build(&p.build) {
        return other(format!("unexpected build {:?} in Apple's catalog", p.build));
    }
    let bdir = dir.join(&p.build);
    fs::create_dir_all(&bdir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(bdir.join(".lock"))?;
    if lock.try_lock().is_err() {
        return other(
            "this installer is already being downloaded (by the app, the command line or an agent)",
        );
    }
    if p.integrity_url.is_empty() {
        return other("Apple's catalog has no integrity data for this installer");
    }
    let integrity = http
        .https_get(&p.integrity_url, 8 << 20)
        .map_err(|e| Error::Other(format!("integrity data: {e}")))?;
    let chunks = parse_integrity(&integrity)?;
    let part = bdir.join(format!("{INSTALLER_FILE}.part"));
    if bdir.join(INSTALLER_FILE).exists() {
        // Verified, but its disc was not written.
        let _ = fs::rename(bdir.join(INSTALLER_FILE), &part);
    }
    let have = fs::metadata(&part).map_or(0, |m| m.len());
    // The package, then the disc written from it: twice its size for a moment.
    let need = (2 * p.size).saturating_sub(have);
    if let Some(free) = rg_core::workspace::free_bytes(&bdir)
        && free < need + (1 << 30)
    {
        return other(format!(
            "not enough free space in {}: {:.1} GB needed, {:.1} GB free",
            dir.display(),
            need as f64 / 1e9,
            free as f64 / 1e9
        ));
    }
    let phase = format!(
        "Downloading the macOS {} installer ({:.1} GB)",
        p.version,
        p.size as f64 / 1e9
    );
    resume_download(http, &p.url, &part, p.size, cancel, &mut |done, total| {
        progress(Progress::new(phase.as_str(), done, total));
    })?;
    if let Err(e) = verify_chunks(&part, &chunks, cancel, &mut |done, total| {
        progress(Progress::new(
            "Checking the package against Apple's integrity data",
            done,
            total,
        ));
    }) {
        if !matches!(e, Error::Cancelled) {
            let _ = fs::remove_file(&part); // a bad download is not resumed; a cancelled check keeps it
        }
        return Err(e);
    }
    progress(Progress::new("Writing the installer disc", 0, 0));
    let disc = bdir.join(DISC_FILE);
    write_installer_disc(&part, &disc, DISC_PIECE, cancel)?; // cancelled: the package stays for next time
    let _ = fs::remove_file(&part);
    let info = serde_json::to_vec_pretty(&p).map_err(|e| Error::Other(e.to_string()))?;
    rg_core::workspace::write_file_atomic(&bdir.join("info.json"), &info, 0o644)?;
    Ok(CachedInstaller {
        product: p,
        path: disc,
    })
}

/// Appends to `part` from where it stopped, with an HTTP Range request. A server that answers
/// 200 instead of 206 restarts the file; a 416 means it is wrong and starts over. Network errors
/// are retried five times.
fn resume_download(
    http: &Http,
    url: &str,
    part: &Path,
    size: u64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let url = match url.strip_prefix("http://") {
        Some(rest) => format!("https://{rest}"), // Apple's CDN serves HTTPS too
        None => url.to_owned(),
    };
    let interrupted = |e: &dyn std::fmt::Display| {
        Error::Other(format!("download interrupted (it resumes next time): {e}"))
    };
    for attempt in 0.. {
        check_cancel(cancel)?;
        let mut have = fs::metadata(part).map_or(0, |m| m.len());
        if have == size {
            return Ok(());
        }
        if have > size {
            let _ = fs::remove_file(part);
            have = 0;
        }
        let mut req = http.download.agent().get(&url);
        if have > 0 {
            req = req.header("Range", format!("bytes={have}-"));
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(e) => {
                if attempt >= 5 {
                    return Err(interrupted(&http_err("download", e)));
                }
                std::thread::sleep(Duration::from_secs(2 * (attempt + 1)));
                continue;
            }
        };
        let content_range = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let append = match resp.status().as_u16() {
            206 if have > 0 && content_range.starts_with(&format!("bytes {have}-")) => true,
            200 => {
                have = 0; // the server ignored the range: start over
                false
            }
            416 => {
                let _ = fs::remove_file(part);
                continue;
            }
            _ => return other(format!("download: {}", status_text(&resp))),
        };
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(part)?;
        let mut last: Option<Instant> = None;
        let res = copy_body(
            &mut resp.into_body().into_reader(),
            &mut f,
            cancel,
            &mut |n| {
                if last.is_none_or(|t| t.elapsed() > Duration::from_millis(250)) {
                    progress(have + n, size);
                    last = Some(Instant::now());
                }
            },
        )
        .and_then(|n| Ok(f.sync_all().map(|()| n)?));
        match res {
            Ok(n) => progress(have + n, size),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) if attempt >= 5 => return Err(interrupted(&e)),
            Err(_) => {}
        }
    }
    other("download: too many attempts")
}

// --- installer disc ----------------------------------------------------------------------------
//
// macOS's cd9660 reads no multi-extent files (one over 4 GiB shows as several 4 GiB ones), so the
// package is stored as consecutive pieces, PKG.000, PKG.001, … that the script joins.

/// The disc's file name in a build's folder.
pub const DISC_FILE: &str = "InstallAssistant.iso";
/// The disc's volume name.
pub const DISC_VOLUME: &str = "OFFLINE";
/// What to type in the recovery's Terminal; " -u" makes it unattended.
pub const DISC_COMMAND: &str = "sh /Volumes/OFFLINE/INSTALL.SH";
/// [`DISC_COMMAND`] for the app to type (layout-safe characters only).
pub const DISC_TYPED: &str = "sh /V*/OFFLINE/I*";

/// The largest sector-aligned size one extent holds.
const DISC_PIECE: u64 = (1 << 32) - 2048;

/// Copies the recovery's own installer app onto the erased disk (the recovery's volume is
/// read-only), joins the package into it as its SharedSupport.dmg and opens it. With -u it
/// first erases the VM's empty disk, and installs with startosinstall, accepting Apple's
/// license, with no questions.
const DISC_SCRIPT: &str = include_str!("install.sh");

/// Holds a shared flock on a disc while a VM uses it; dropping it releases the disc.
#[derive(Debug)]
pub struct DiscLease(#[expect(dead_code, reason = "held for its lock")] File);

/// Marks a disc as in use by a running VM, seen from any process (the app or the CLI).
pub fn use_disc(path: &Path) -> Result<DiscLease> {
    let f = File::open(path)?;
    f.lock_shared()?;
    Ok(DiscLease(f))
}

/// Whether a running VM holds the disc (see [`use_disc`]).
pub fn disc_in_use(path: &Path) -> bool {
    File::open(path).is_ok_and(|f| f.try_lock().is_err())
}

/// 32 KiB kept for the script, so a newer one fits in place.
const DISC_SCRIPT_SECTORS: u32 = 16;
const SCRIPT_LBA: u32 = 21;

fn both32(b: &mut [u8], v: u32) {
    b[..4].copy_from_slice(&v.to_le_bytes());
    b[4..8].copy_from_slice(&v.to_be_bytes());
}

fn both16(b: &mut [u8], v: u16) {
    b[..2].copy_from_slice(&v.to_le_bytes());
    b[2..4].copy_from_slice(&v.to_be_bytes());
}

fn record(name: &[u8], lba: u32, size: u32, dir: bool) -> Vec<u8> {
    let mut r = vec![0u8; 33 + name.len() + (1 - name.len() % 2)];
    r[0] = r.len() as u8;
    both32(&mut r[2..], lba);
    both32(&mut r[10..], size);
    r[18..21].copy_from_slice(&[120, 1, 1]); // 2020-01-01: a fixed date keeps the head reproducible
    if dir {
        r[25] = 2;
    }
    both16(&mut r[28..], 1);
    r[32] = name.len() as u8;
    r[33..33 + name.len()].copy_from_slice(name);
    r
}

/// Everything before the package on a disc for a package of `pkg_size` bytes: the volume
/// descriptors (sectors 16, 17), path tables (18, 19), the root directory (20) and the script
/// (21–36). The pieces follow back to back from the returned LBA, so the package is one
/// contiguous copy. It depends only on `pkg_size`, so a cached disc's head can be rewritten with
/// a newer script. Returns (head, package LBA, total sectors).
fn disc_head(pkg_size: u64, piece: u64) -> Result<(Vec<u8>, u32, u32)> {
    if DISC_SCRIPT.len() > DISC_SCRIPT_SECTORS as usize * 2048 {
        return other("installer disc: script too large");
    }
    let mut files = vec![(
        b"INSTALL.SH;1".to_vec(),
        SCRIPT_LBA,
        DISC_SCRIPT.len() as u32,
    )];
    let pkg_lba = SCRIPT_LBA + DISC_SCRIPT_SECTORS;
    let mut next = pkg_lba;
    let (mut off, mut n) = (0u64, 0);
    while off < pkg_size {
        let size = (pkg_size - off).min(piece);
        files.push((format!("PKG.{n:03};1").into_bytes(), next, size as u32));
        next += size.div_ceil(2048) as u32;
        off += piece;
        n += 1;
    }
    let total = next;

    let mut root = [
        record(b"\x00", 20, 2048, true),
        record(b"\x01", 20, 2048, true),
    ]
    .concat();
    for (name, lba, size) in &files {
        root.extend(record(name, *lba, *size, false));
    }
    if root.len() > 2048 {
        return other("installer disc: too many pieces");
    }

    let mut head = vec![0u8; pkg_lba as usize * 2048];
    {
        let pvd = &mut head[16 * 2048..17 * 2048];
        pvd[0] = 1;
        pvd[6] = 1;
        pvd[1..6].copy_from_slice(b"CD001");
        pvd[8..72].fill(b' ');
        pvd[40..40 + DISC_VOLUME.len()].copy_from_slice(DISC_VOLUME.as_bytes());
        both32(&mut pvd[80..], total);
        both16(&mut pvd[120..], 1);
        both16(&mut pvd[124..], 1);
        both16(&mut pvd[128..], 2048);
        both32(&mut pvd[132..], 10); // one path table entry: the root
        pvd[140..144].copy_from_slice(&18u32.to_le_bytes());
        pvd[148..152].copy_from_slice(&19u32.to_be_bytes());
        let r = record(b"\x00", 20, 2048, true);
        pvd[156..156 + r.len()].copy_from_slice(&r);
        pvd[190..813].fill(b' ');
        for at in [813, 830, 847, 864] {
            // creation, modification, expiration, effective: unset
            pvd[at..at + 16].fill(b'0');
        }
        pvd[881] = 1;
    }
    {
        let term = &mut head[17 * 2048..];
        term[0] = 255;
        term[6] = 1;
        term[1..6].copy_from_slice(b"CD001");
    }
    head[18 * 2048] = 1;
    head[18 * 2048 + 2..18 * 2048 + 6].copy_from_slice(&20u32.to_le_bytes());
    head[18 * 2048 + 6..18 * 2048 + 8].copy_from_slice(&1u16.to_le_bytes());
    head[19 * 2048] = 1;
    head[19 * 2048 + 2..19 * 2048 + 6].copy_from_slice(&20u32.to_be_bytes());
    head[19 * 2048 + 6..19 * 2048 + 8].copy_from_slice(&1u16.to_be_bytes());
    head[20 * 2048..20 * 2048 + root.len()].copy_from_slice(&root);
    let s = SCRIPT_LBA as usize * 2048;
    head[s..s + DISC_SCRIPT.len()].copy_from_slice(DISC_SCRIPT.as_bytes());
    Ok((head, pkg_lba, total))
}

/// Writes the disc `iso` from the package `pkg`; cancelling stops it and leaves no disc.
fn write_installer_disc(pkg: &Path, iso: &Path, piece: u64, cancel: &AtomicBool) -> Result<()> {
    let mut src = File::open(pkg)?;
    let (head, pkg_lba, total) = disc_head(src.metadata()?.len(), piece)?;
    let tmp = iso.with_extension("iso.tmp");
    let res = (|| {
        let mut out = File::create(&tmp)?;
        out.write_all(&head)?;
        out.seek(SeekFrom::Start(u64::from(pkg_lba) * 2048))?;
        // 1 GiB at a time: copy_file_range stays in the kernel, and a cancel is seen.
        loop {
            check_cancel(cancel)?;
            if io::copy(&mut (&mut src).take(1 << 30), &mut out)? < 1 << 30 {
                break;
            }
        }
        out.set_len(u64::from(total) * 2048)?;
        out.sync_all()?;
        fs::rename(&tmp, iso)?;
        Ok(())
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res.map_err(|e| match e {
        Error::Io(e) => Error::Other(format!("installer disc: {e}")),
        e => e,
    })
}

/// Rewrites a cached disc's head when this version's script differs, so discs written by an
/// older version get the current one.
pub fn refresh_installer_disc(c: &CachedInstaller) -> Result<()> {
    refresh_disc(c, DISC_PIECE)
}

fn refresh_disc(c: &CachedInstaller, piece: u64) -> Result<()> {
    let (head, _, total) = disc_head(c.product.size, piece)?;
    let mut f = OpenOptions::new().read(true).write(true).open(&c.path)?;
    if f.metadata()?.len() != u64::from(total) * 2048 {
        return other(
            "installer disc: unexpected size; delete the installer and download it again",
        );
    }
    let mut old = vec![0; head.len()];
    f.read_exact(&mut old)?;
    if old != head {
        f.write_all_at(&head, 0)?;
        f.sync_all()?;
    }
    Ok(())
}

/// "Sequoia" for a version such as "15.8.1".
pub fn installer_name(version: &str) -> String {
    let major = major_of(version);
    INSTALLER_MAJOR
        .iter()
        .find(|(_, m)| *m == major)
        .map_or_else(
            || version.to_owned(),
            |(short, _)| rg_core::config::version_display(short).to_owned(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunklist::tests::{integrity, tempdir};
    use crate::testserver::{Reply, Server};

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    const SAMPLE_CATALOG: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>Products</key><dict>
  <key>140-1</key><dict>
    <key>ExtendedMetaInfo</key><dict><key>InstallAssistantPackageIdentifiers</key><dict><key>SharedSupport</key><string>com.apple.pkg.InstallAssistant.macOSSequoia</string></dict></dict>
    <key>Distributions</key><dict><key>English</key><string>https://swdist.apple.com/140-1.English.dist</string></dict>
    <key>Packages</key><array>
      <dict><key>URL</key><string>https://swcdn.apple.com/x/BuildManifest.plist</string><key>Size</key><integer>10</integer></dict>
      <dict><key>URL</key><string>https://swcdn.apple.com/x/InstallAssistant.pkg</string><key>Size</key><integer>14930000000</integer>
        <key>IntegrityDataURL</key><string>https://swcdn.apple.com/x/InstallAssistant.pkg.integrityDataV1</string></dict>
    </array>
  </dict>
  <key>041-2</key><dict><key>Packages</key><array/></dict>
</dict></dict></plist>"#;

    #[test]
    fn catalog_parsing() -> TestResult {
        let got = catalog_installers(SAMPLE_CATALOG.as_bytes())?;
        assert_eq!(got.len(), 1);
        let (p, dist) = &got[0];
        assert_eq!((p.product_id.as_str(), p.size), ("140-1", 14_930_000_000));
        assert!(p.integrity_url.ends_with(".integrityDataV1") && dist.ends_with(".dist"));
        let (v, b) =
            dist_info(b"<dict><key>BUILD</key><string>24H32</string><key>VERSION</key><string>15.8.1</string></dict>");
        assert_eq!((v.as_str(), b.as_str()), ("15.8.1", "24H32"));
        Ok(())
    }

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(version_cmp("15.9", "15.10"), Ordering::Less);
        assert_ne!(version_cmp("26.7.1", "26.7"), Ordering::Less);
        assert_eq!(version_cmp("14.8.9", "15.0"), Ordering::Less);
    }

    #[test]
    fn full_installers_exist_from_big_sur_on() {
        assert!(
            !installer_supported("catalina")
                && installer_supported("big-sur")
                && installer_supported("tahoe")
        );
        assert_eq!(installer_name("15.8.1"), "Sequoia");
    }

    #[test]
    fn resume_continues_a_partial_download() -> TestResult {
        let payload = b"macOS installer payload ".repeat(50000); // ~1.2 MB
        let data = payload.clone();
        let srv = Server::https(move |r| Reply::content(r, &data))?;
        let http = Http::for_tests(&srv.cert_der, "");
        let part = tempdir()?.join("InstallAssistant.pkg.part");
        fs::write(&part, &payload[..400_000])?; // an interrupted download
        resume_download(
            &http,
            &srv.url("/"),
            &part,
            payload.len() as u64,
            &AtomicBool::new(false),
            &mut |_, _| {},
        )?;
        assert!(
            fs::read(&part)? == payload,
            "the resumed file must equal the original"
        );
        let chunks = parse_integrity(&integrity(&payload, 65536))?;
        verify_chunks(&part, &chunks, &AtomicBool::new(false), &mut |_, _| {})?;
        Ok(())
    }

    #[test]
    fn resume_restarts_a_file_longer_than_the_package() -> TestResult {
        let payload = b"0123456789".repeat(1000);
        let data = payload.clone();
        let srv = Server::https(move |r| Reply::content(r, &data))?;
        let http = Http::for_tests(&srv.cert_der, "");
        let part = tempdir()?.join("p.part");
        fs::write(&part, b"x".repeat(payload.len() + 5))?;
        resume_download(
            &http,
            &srv.url("/"),
            &part,
            payload.len() as u64,
            &AtomicBool::new(false),
            &mut |_, _| {},
        )?;
        assert!(fs::read(&part)? == payload);
        Ok(())
    }

    fn le32(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    fn be32(b: &[u8], at: usize) -> u32 {
        u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    #[test]
    fn installer_disc_and_cache() -> TestResult {
        let root = tempdir()?;
        let cache = installers_dir(&root);
        let dir = cache.join("24H32");
        fs::create_dir_all(&dir)?;
        let payload = b"0123456789abcdef".repeat(700); // 11200 bytes: pieces of 4096, 4096, 3008
        let pkg = dir.join(INSTALLER_FILE);
        fs::write(&pkg, &payload)?;
        let iso = dir.join(DISC_FILE);
        let res = write_installer_disc(&pkg, &iso, 4096, &AtomicBool::new(true));
        assert!(
            res.is_err() && !iso.exists() && pkg.exists(),
            "a cancelled write leaves no disc, keeps the package"
        );
        write_installer_disc(&pkg, &iso, 4096, &AtomicBool::new(false))?;

        // Read it back the way a cd9660 driver does: PVD at sector 16 → root directory → files.
        let b = fs::read(&iso)?;
        let pvd = &b[16 * 2048..];
        assert!(
            &pvd[1..6] == b"CD001"
                && pvd[0] == 1
                && String::from_utf8_lossy(&pvd[40..72]).trim() == DISC_VOLUME
                && le32(pvd, 80) as usize * 2048 == b.len()
                && b[17 * 2048] == 255,
            "volume descriptors"
        );
        let root_lba = le32(pvd, 158) as usize;
        let mut files = std::collections::HashMap::new();
        let mut pieces = Vec::new();
        let mut d = &b[root_lba * 2048..root_lba * 2048 + 2048];
        while d[0] != 0 {
            let name = String::from_utf8_lossy(&d[33..33 + d[32] as usize]).into_owned();
            let (lba, size) = (le32(d, 2), le32(d, 10));
            assert!(
                be32(d, 6) == lba && be32(d, 14) == size,
                "{name:?}: both-endian fields differ"
            );
            let content = b[lba as usize * 2048..lba as usize * 2048 + size as usize].to_vec();
            if name.starts_with("PKG.") {
                pieces.extend_from_slice(&content);
            }
            files.insert(name, content);
            d = &d[d[0] as usize..];
        }
        assert!(
            files.len() == 6 && files.contains_key("PKG.002;1") && pieces == payload,
            "pieces join into the package"
        );
        let sh = String::from_utf8_lossy(&files["INSTALL.SH;1"]).into_owned();
        assert!(
            sh.contains("SharedSupport.dmg") && sh.contains("[Pp][Kk][Gg]"),
            "script: {sh}"
        );

        fs::remove_file(&pkg)?;
        let info = InstallerProduct {
            version: "15.8.1".into(),
            build: "24H32".into(),
            size: payload.len() as u64,
            ..Default::default()
        };
        fs::write(dir.join("info.json"), serde_json::to_vec(&info)?)?;
        let c = cached_installer_for(&cache, "sequoia");
        assert_eq!(c.as_ref().map(|c| c.path.clone()), Some(iso.clone()));
        assert!(
            cached_installer_for(&cache, "sonoma").is_none(),
            "a Sequoia installer is not a Sonoma one"
        );
        let Some(c) = c else { return Ok(()) };

        // A disc from an older version, with another script, gets this version's head in place.
        OpenOptions::new()
            .write(true)
            .open(&iso)?
            .write_all_at(b"#!/bin/sh\necho old\n", 21 * 2048)?;
        refresh_disc(&c, 4096)?;
        assert!(
            fs::read(&iso)? == b,
            "the refreshed disc must equal a new one"
        );

        let lease = use_disc(&iso)?;
        assert!(
            remove_installer(&cache, "24H32").is_err(),
            "an attached installer must not be deleted"
        );
        drop(lease);
        remove_installer(&cache, "24H32")?;
        assert!(!dir.exists(), "removed after the VM stops");
        assert!(remove_installer(&cache, "../x").is_err());
        Ok(())
    }

    #[test]
    fn remove_installer_stays_in_the_cache() -> TestResult {
        let tmp = tempdir()?;
        let cache = tmp.join("ws/.installers");
        let outside = tmp.join("outside");
        fs::create_dir_all(&cache)?;
        fs::create_dir_all(&outside)?;
        fs::write(outside.join("keep"), b"")?;
        std::os::unix::fs::symlink(&outside, cache.join("24H33"))?;
        let abs = outside.to_string_lossy().into_owned();
        for b in [
            "..",
            ".",
            "a/../..",
            "../x",
            abs.as_str(),
            "24H33",
            "",
            "24H32/..",
        ] {
            assert!(
                remove_installer(&cache, b).is_err(),
                "{b:?} must be refused"
            );
        }
        assert!(outside.join("keep").exists() && cache.exists());

        fs::create_dir_all(cache.join("25A5279m"))?;
        remove_installer(&cache, "25A5279m")?;
        assert!(!cache.join("25A5279m").exists());
        Ok(())
    }

    #[test]
    fn info_json_matches_go() -> TestResult {
        let p = InstallerProduct {
            build: "24H32".into(),
            size: 7,
            ..Default::default()
        };
        let go = "{\n  \"product_id\": \"\",\n  \"version\": \"\",\n  \"build\": \"24H32\",\n  \"url\": \"\",\n  \"size\": 7,\n  \"integrity_url\": \"\"\n}";
        assert_eq!(String::from_utf8(serde_json::to_vec_pretty(&p)?)?, go);
        Ok(())
    }
}

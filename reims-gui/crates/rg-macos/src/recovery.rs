//! Apple's recovery API (ported from acidanthera's macrecovery, BSD-3-Clause, Copyright (c) 2019
//! vit9696) and turning the recovery into the VM's install media (`workspace.go` DownloadMedia).

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use rsa::RsaPublicKey;

use crate::chunklist::{apple_efi_key, verify_image_with};
use crate::http::{Http, copy_body, http_err, status_text};
use crate::{Error, Progress, Result, check_cancel, other};
use rg_core::workspace::{GIB, MEDIA_FILE, MEDIA_MARKER, MIN_FREE_GIB, STAGING_DIR};

/// What Apple's recovery server should return for a macOS version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryProduct {
    /// The board ID sent as `bid`.
    pub board_id: &'static str,
    /// The MLB sent as `sn`.
    pub mlb: &'static str,
    /// `default` or `latest`.
    pub os_type: &'static str,
}

const fn product(
    board_id: &'static str,
    mlb: &'static str,
    os_type: &'static str,
) -> RecoveryProduct {
    RecoveryProduct {
        board_id,
        mlb,
        os_type,
    }
}

/// macrecovery's product table, by the app's short version names.
pub const RECOVERY_PRODUCTS: [(&str, RecoveryProduct); 9] = [
    (
        "high-sierra",
        product("Mac-7BA5B2D9E42DDD94", "00000000000J80300", "default"),
    ),
    (
        "mojave",
        product("Mac-7BA5B2DFE22DDD8C", "00000000000KXPG00", "default"),
    ),
    (
        "catalina",
        product("Mac-00BE6ED71E35EB86", "00000000000000000", "default"),
    ),
    (
        "big-sur",
        product("Mac-2BD1B31983FE1663", "00000000000000000", "default"),
    ),
    (
        "monterey",
        product("Mac-B809C3757DA9BB8D", "00000000000000000", "latest"),
    ),
    (
        "ventura",
        product("Mac-4B682C642B45593E", "00000000000000000", "latest"),
    ),
    (
        "sonoma",
        product("Mac-827FAC58A8FDFA22", "00000000000000000", "default"),
    ),
    (
        "sequoia",
        product("Mac-7BA5B2D9E42DDD94", "00000000000000000", "default"),
    ),
    (
        "tahoe",
        product("Mac-CFF7D910A743CAAF", "00000000000000000", "latest"),
    ),
];

/// The recovery product of a macOS version, if it has one.
pub fn recovery_product(version: &str) -> Option<RecoveryProduct> {
    RECOVERY_PRODUCTS
        .iter()
        .find(|(v, _)| *v == version)
        .map(|(_, p)| *p)
}

const RECOVERY_HOST: &str = "osrecovery.apple.com";
/// Keys macrecovery requires in the RecoveryImage response.
const REQUIRED_KEYS: [&str; 7] = ["AP", "AU", "AH", "AT", "CU", "CH", "CT"];

/// Media preparation phases.
pub const STAGE_CHUNKLIST: &str = "Downloading chunklist";
/// Downloading the recovery DMG.
pub const STAGE_IMAGE: &str = "Downloading image";
/// Checking the DMG against the signed chunklist.
pub const STAGE_VERIFY: &str = "Verifying signature and hashes";
/// `qemu-img convert`.
pub const STAGE_CONVERT: &str = "Converting to raw image";

/// Downloads the chunklist and the DMG for `version` into `dir` and returns their paths
/// (`BaseSystem.dmg`, `BaseSystem.chunklist`: names from the server never reach the file
/// system). Each file goes through a `.part` file renamed when complete; on any error nothing is
/// left in `dir`. Progress is in bytes, with [`STAGE_CHUNKLIST`] then [`STAGE_IMAGE`].
pub fn download_recovery(
    http: &Http,
    version: &str,
    dir: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<(PathBuf, PathBuf)> {
    let Some(p) = recovery_product(version) else {
        return other(format!("recovery: unknown macOS version {version:?}"));
    };
    check_cancel(cancel)?;
    let session = session(http)?;
    check_cancel(cancel)?;
    let info = image_info(http, &session, &p)?;
    fs::create_dir_all(dir)?;
    let cnk = save(
        http,
        &info.cu,
        &info.ct,
        &dir.join("BaseSystem.chunklist"),
        STAGE_CHUNKLIST,
        cancel,
        progress,
    )?;
    match save(
        http,
        &info.au,
        &info.at,
        &dir.join("BaseSystem.dmg"),
        STAGE_IMAGE,
        cancel,
        progress,
    ) {
        Ok(dmg) => Ok((dmg, cnk)),
        Err(e) => {
            let _ = fs::remove_file(&cnk);
            Err(e)
        }
    }
}

fn recovery_request<B>(b: ureq::RequestBuilder<B>, host: &str) -> ureq::RequestBuilder<B> {
    b.header("Host", host)
        .header("Connection", "close")
        // urllib sends identity; it also keeps the image from being transparently decoded.
        .header("Accept-Encoding", "identity")
}

/// The "session=..." cookie from the server root.
fn session(http: &Http) -> Result<String> {
    let url = format!("{}/", http.recovery_base);
    let resp = recovery_request(http.recovery_api.agent().get(&url), RECOVERY_HOST)
        .call()
        .map_err(|e| http_err("recovery: session", e))?;
    if resp.status() != 200 {
        return other(format!(
            "recovery: session: unexpected status {}",
            status_text(&resp)
        ));
    }
    let cookie = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|h| h.split("; "))
        .find(|c| c.starts_with("session="))
        .map(str::to_owned);
    let _ = resp.into_body().into_reader().read_to_end(&mut Vec::new());
    cookie.ok_or_else(|| Error::Other("recovery: no session cookie in server response".into()))
}

struct ImageInfo {
    au: String,
    at: String,
    cu: String,
    ct: String,
}

/// POSTs the board identity and parses the "key: value" reply.
fn image_info(http: &Http, session: &str, p: &RecoveryProduct) -> Result<ImageInfo> {
    let body = [
        format!("cid={}", recovery_id(16)?),
        format!("sn={}", p.mlb),
        format!("bid={}", p.board_id),
        format!("k={}", recovery_id(64)?),
        format!("fg={}", recovery_id(64)?),
        format!("os={}", p.os_type),
    ]
    .join("\n");
    let url = format!("{}/InstallationPayload/RecoveryImage", http.recovery_base);
    let resp = recovery_request(http.recovery_api.agent().post(&url), RECOVERY_HOST)
        .header("Cookie", session)
        .header("Content-Type", "text/plain")
        .send(body.as_bytes())
        .map_err(|e| http_err("recovery: image info", e))?;
    if resp.status() != 200 {
        return other(format!(
            "recovery: image info: unexpected status {}",
            status_text(&resp)
        ));
    }
    let mut out = Vec::new();
    resp.into_body()
        .into_reader()
        .take(1 << 20)
        .read_to_end(&mut out)
        .map_err(|e| Error::Other(format!("recovery: image info: {e}")))?;
    let out = String::from_utf8_lossy(&out);
    // Like Python's line.split(': '): only lines with exactly one separator count.
    let info: Vec<(&str, &str)> = out
        .split('\n')
        .filter_map(|l| {
            let mut kv = l.split(": ");
            match (kv.next(), kv.next(), kv.next()) {
                (Some(k), Some(v), None) => Some((k, v)),
                _ => None,
            }
        })
        .collect();
    let get = |key: &str| {
        info.iter()
            .rev()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| (*v).to_owned())
    };
    if let Some(missing) = REQUIRED_KEYS.iter().find(|k| get(k).is_none()) {
        return other(format!("recovery: server response missing key {missing}"));
    }
    let get = |key: &str| get(key).unwrap_or_default();
    Ok(ImageInfo {
        au: get("AU"),
        at: get("AT"),
        cu: get("CU"),
        ct: get("CT"),
    })
}

/// `n` random uppercase hex digits.
fn recovery_id(n: usize) -> Result<String> {
    let mut b = vec![0u8; n];
    File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b.iter()
        .map(|x| char::from(b"0123456789ABCDEF"[usize::from(x & 0xF)]))
        .collect())
}

/// Downloads `url` into `dst` through an exclusively created `.part` file.
fn save(
    http: &Http,
    url: &str,
    token: &str,
    dst: &Path,
    stage: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<PathBuf> {
    let name = dst
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let uri: ureq::http::Uri = url
        .parse()
        .map_err(|e| Error::Other(format!("recovery: bad {stage} URL: {e}")))?;
    let host = uri.host().unwrap_or_default().to_owned();
    let resp = recovery_request(http.recovery_download.agent().get(url), &host)
        .header("Cookie", format!("AssetToken={token}"))
        .call()
        .map_err(|e| http_err(&format!("recovery: download {name}"), e))?;
    if resp.status() != 200 {
        return other(format!(
            "recovery: download {name}: unexpected status {}",
            status_text(&resp)
        ));
    }
    let total = resp.body().content_length().unwrap_or(0);
    let (mut f, part) = create_part(dst, &name)?;
    let res = (|| {
        progress(Progress::new(stage, 0, total));
        let n = copy_body(
            &mut resp.into_body().into_reader(),
            &mut f,
            cancel,
            &mut |n| {
                progress(Progress::new(stage, n, total));
            },
        )
        .map_err(|e| match e {
            Error::Io(e) => Error::Other(format!("recovery: download {name}: {e}")),
            e => e,
        })?;
        if total > 0 && n != total {
            return other(format!(
                "recovery: download {name}: got {n} of {total} bytes"
            ));
        }
        f.sync_all()?;
        fs::rename(&part, dst)?;
        Ok(dst.to_owned())
    })();
    if res.is_err() {
        let _ = fs::remove_file(&part);
    }
    res
}

/// `<name>.<random>.part` next to `dst`, created exclusively (Go's os.CreateTemp).
fn create_part(dst: &Path, name: &str) -> Result<(File, PathBuf)> {
    let dir = dst.parent().unwrap_or(Path::new("."));
    loop {
        let part = dir.join(format!("{name}.{}.part", recovery_id(10)?));
        match OpenOptions::new().write(true).create_new(true).open(&part) {
            Ok(f) => return Ok((f, part)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

/// Downloads, verifies and publishes the recovery for `version` as `<dir>/BaseSystem.img`
/// (`Workspace.DownloadMedia` without its lock: the caller holds the VM folder's lock). On any
/// failure the previous media and its version marker stay intact. `log` gets qemu-img's output.
pub fn download_media(
    http: &Http,
    dir: &Path,
    version: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
    log: &mut dyn FnMut(&str),
) -> Result<()> {
    download_media_with(http, dir, version, &apple_efi_key()?, cancel, progress, log)
}

/// Removes the staging folder however `download_media` ends.
struct Staging(PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn download_media_with(
    http: &Http,
    dir: &Path,
    version: &str,
    key: &RsaPublicKey,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
    log: &mut dyn FnMut(&str),
) -> Result<()> {
    fs::create_dir_all(dir)?;
    if let Some(free) = rg_core::workspace::free_bytes(dir)
        && free < MIN_FREE_GIB * GIB
    {
        return other(format!(
            "not enough free space: {MIN_FREE_GIB} GiB needed and {:.1} GiB available",
            free as f64 / GIB as f64
        ));
    }
    let staging = Staging(dir.join(STAGING_DIR));
    let _ = fs::remove_dir_all(&staging.0);
    fs::create_dir(&staging.0)?;

    progress(Progress::new(STAGE_CHUNKLIST, 0, 0));
    let (dmg, cnk) =
        download_recovery(http, version, &staging.0, cancel, progress).map_err(|e| match e {
            Error::Cancelled => e,
            e => Error::Other(format!("download failed: {e}")),
        })?;
    verify_image_with(&dmg, &cnk, key, &mut |done, total| {
        progress(Progress::new(STAGE_VERIFY, done, total))
    })
    .map_err(|e| Error::Other(format!("verification failed: {e}")))?;

    progress(Progress::new(STAGE_CONVERT, 0, 1000));
    let tmp = staging.0.join(MEDIA_FILE);
    let mut each = |line: &str| match parse_convert_progress(line) {
        // Percentages go only to the bar.
        Some(pct) => progress(Progress::new(STAGE_CONVERT, (pct * 10.0) as u64, 1000)),
        None => log(line),
    };
    let mut cmd = Command::new("qemu-img");
    cmd.args(["convert", "-p", "-f", "dmg", "-O", "raw"])
        .arg(&dmg)
        .arg(&tmp);
    run_stream(cmd, cancel, &mut each).map_err(|e| match e {
        Error::Cancelled => e,
        e => Error::Other(format!(
            "conversion failed (if qemu-img does not support this compression, install dmg2img): {e}"
        )),
    })?;
    // The old marker goes before the rename: a stale version never labels the new media.
    let _ = fs::remove_file(dir.join(MEDIA_MARKER));
    fs::rename(&tmp, dir.join(MEDIA_FILE))?;
    // After the rename the new media is already valid; failing here only leaves the version "unknown".
    if let Err(e) = fs::write(dir.join(MEDIA_MARKER), format!("{version}\n")) {
        log(&format!("warning: could not record the media version: {e}"));
    }
    Ok(())
}

/// The percentage printed by `qemu-img convert -p`, e.g. "(42.50/100%)".
fn parse_convert_progress(line: &str) -> Option<f64> {
    let rest = &line[line.find('(')? + 1..];
    rest[..rest.find("/100%)")?].parse().ok()
}

/// Runs `cmd` with stdout and stderr merged, passing each non-empty line (split on `\n` or `\r`,
/// as progress bars rewrite the line) to `each`; cancelling kills it (Go's runStream).
fn run_stream(mut cmd: Command, cancel: &AtomicBool, each: &mut dyn FnMut(&str)) -> Result<()> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let (reader, writer) = std::io::pipe()?;
    cmd.stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer);
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Other(format!("{name}: {e}")))?;
    drop(cmd); // closes our copies of the pipe's write end, so the reader sees the end
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut r = BufReader::new(reader);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            // Split on \n here and on \r below.
            match r.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    for part in String::from_utf8_lossy(&buf).split(['\r', '\n']) {
                        if !part.trim().is_empty() && tx.send(part.trim().to_owned()).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });
    let mut last = String::new();
    let status = loop {
        while let Ok(line) = rx.try_recv() {
            each(&line);
            last = line;
        }
        if crate::check_cancel(cancel).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Cancelled);
        }
        if let Some(st) = child.try_wait()? {
            break st;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    // The rest of the output, until the pipe closes.
    for line in rx {
        each(&line);
        last = line;
    }
    if status.success() {
        Ok(())
    } else if last.is_empty() {
        other(format!("{name}: {status}"))
    } else {
        other(format!("{name}: {status} ({last})"))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::chunklist::tests::{build_chunklist, tempdir, test_key};
    use crate::testserver::{Reply, Request, Server};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    #[derive(Default)]
    pub(crate) struct State {
        pub body: String,
        pub cookies: HashMap<String, String>,
        pub status: u16,
        pub short_dmg: bool,
        pub chunklist: Vec<u8>,
        pub dmg: Vec<u8>,
        pub errors: Vec<String>,
        base: String,
    }

    pub(crate) struct Fake {
        pub state: Arc<Mutex<State>>,
        pub http: Http,
    }

    impl Fake {
        pub fn st(&self) -> std::sync::MutexGuard<'_, State> {
            self.state.lock().unwrap_or_else(|e| e.into_inner())
        }
    }

    fn serve(st: &Mutex<State>, r: &Request) -> Reply {
        let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
        s.cookies
            .insert(r.path.clone(), r.header("cookie").to_owned());
        if r.header("user-agent") != "InternetRecovery/1.0" {
            s.errors.push(format!(
                "{}: User-Agent {:?}",
                r.path,
                r.header("user-agent")
            ));
        }
        match r.path.as_str() {
            "/" => {
                if r.header("host") != "osrecovery.apple.com" {
                    s.errors
                        .push(format!("session Host {:?}", r.header("host")));
                }
                Reply::new(
                    200,
                    vec![(
                        "Set-Cookie".into(),
                        "session=abc123; Path=/; HttpOnly".into(),
                    )],
                    Vec::new(),
                )
            }
            "/InstallationPayload/RecoveryImage" => {
                if r.method != "POST" || r.header("content-type") != "text/plain" {
                    s.errors.push(format!(
                        "info: method {} content-type {:?}",
                        r.method,
                        r.header("content-type")
                    ));
                }
                s.body = String::from_utf8_lossy(&r.body).into_owned();
                if s.status != 0 {
                    return Reply::new(s.status, Vec::new(), Vec::new());
                }
                let u = &s.base;
                Reply::ok(
                    format!(
                        "AP: 041-12345\nAU: {u}/dl/BaseSystem.dmg\nAH: AAAA\nAT: imgtok\nCU: {u}/dl/BaseSystem.chunklist\nCH: BBBB\nCT: cnktok\n"
                    )
                    .into_bytes(),
                )
            }
            "/dl/BaseSystem.chunklist" => Reply::ok(s.chunklist.clone()),
            "/dl/BaseSystem.dmg" if s.short_dmg => Reply::new(
                200,
                vec![("Content-Length".into(), "1000".into())],
                b"short".to_vec(),
            ),
            "/dl/BaseSystem.dmg" => Reply::ok(s.dmg.clone()),
            _ => Reply::new(404, Vec::new(), Vec::new()),
        }
    }

    pub(crate) fn fake_recovery() -> std::result::Result<Fake, Box<dyn std::error::Error>> {
        let state = Arc::new(Mutex::new(State {
            chunklist: b"CNKL-data".to_vec(),
            dmg: vec![b'D'; 3 << 20],
            ..State::default()
        }));
        let st = state.clone();
        let srv = Server::http(move |r| serve(&st, r))?;
        state.lock().unwrap_or_else(|e| e.into_inner()).base = srv.base.clone();
        Ok(Fake {
            state,
            http: Http::for_tests(&[], &srv.base),
        })
    }

    fn entries(dir: &Path) -> std::io::Result<Vec<String>> {
        fs::read_dir(dir)?
            .map(|e| Ok(e?.file_name().to_string_lossy().into_owned()))
            .collect()
    }

    fn body_matches(body: &str, bid: &str, os: &str) -> bool {
        let lines: Vec<&str> = body.split('\n').collect();
        let hex = |s: &str, p: &str, n: usize| {
            s.strip_prefix(p).is_some_and(|h| {
                h.len() == n
                    && h.bytes()
                        .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
            })
        };
        lines.len() == 6
            && hex(lines[0], "cid=", 16)
            && lines[1] == "sn=00000000000000000"
            && lines[2] == format!("bid={bid}")
            && hex(lines[3], "k=", 64)
            && hex(lines[4], "fg=", 64)
            && lines[5] == format!("os={os}")
    }

    #[test]
    fn recovery_download() -> TestResult {
        for (version, bid, os) in [
            ("sonoma", "Mac-827FAC58A8FDFA22", "default"),
            ("tahoe", "Mac-CFF7D910A743CAAF", "latest"),
        ] {
            let f = fake_recovery()?;
            let dir = tempdir()?.join("out");
            let mut last: HashMap<String, f64> = HashMap::new();
            let never = AtomicBool::new(false);
            let (dmg, cnk) = download_recovery(&f.http, version, &dir, &never, &mut |p| {
                last.insert(p.phase.clone(), p.pct().unwrap_or(-1.0));
            })?;
            assert_eq!(
                (dmg.clone(), cnk.clone()),
                (dir.join("BaseSystem.dmg"), dir.join("BaseSystem.chunklist"))
            );
            let s = f.st();
            assert_eq!(fs::read(&dmg)?, s.dmg, "dmg content");
            assert_eq!(fs::read(&cnk)?, s.chunklist, "chunklist content");
            assert_eq!(entries(&dir)?.len(), 2);
            assert_eq!(
                (last.get(STAGE_IMAGE), last.get(STAGE_CHUNKLIST)),
                (Some(&100.0), Some(&100.0))
            );
            assert!(body_matches(&s.body, bid, os), "POST body:\n{}", s.body);
            for (p, c) in [
                ("/InstallationPayload/RecoveryImage", "session=abc123"),
                ("/dl/BaseSystem.dmg", "AssetToken=imgtok"),
                ("/dl/BaseSystem.chunklist", "AssetToken=cnktok"),
            ] {
                assert_eq!(s.cookies.get(p).map(String::as_str), Some(c), "{p}");
            }
            assert!(s.errors.is_empty(), "{:?}", s.errors);
        }
        Ok(())
    }

    #[test]
    fn recovery_unknown_version() -> TestResult {
        let e = download_recovery(
            &Http::default(),
            "leopard",
            &tempdir()?,
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert!(e.is_err_and(|e| e.to_string().contains("unknown")));
        Ok(())
    }

    #[test]
    fn recovery_forbidden() -> TestResult {
        let f = fake_recovery()?;
        f.st().status = 403;
        let dir = tempdir()?;
        let e = download_recovery(
            &f.http,
            "sonoma",
            &dir,
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert!(e.is_err_and(|e| e.to_string().contains("403")));
        assert!(entries(&dir)?.is_empty(), "leftover files");
        Ok(())
    }

    #[test]
    fn recovery_interrupted_leaves_nothing() -> TestResult {
        let f = fake_recovery()?;
        f.st().short_dmg = true;
        let dir = tempdir()?;
        assert!(
            download_recovery(
                &f.http,
                "sonoma",
                &dir,
                &AtomicBool::new(false),
                &mut |_| {}
            )
            .is_err()
        );
        assert!(
            entries(&dir)?.is_empty(),
            "leftover files {:?}",
            entries(&dir)?
        );
        Ok(())
    }

    #[test]
    fn recovery_cancelled() -> TestResult {
        let f = fake_recovery()?;
        let dir = tempdir()?;
        let res = download_recovery(&f.http, "sonoma", &dir, &AtomicBool::new(true), &mut |_| {});
        assert!(matches!(res, Err(Error::Cancelled)));
        assert!(entries(&dir)?.is_empty());
        Ok(())
    }

    /// Names in the server's URLs must never reach the file system.
    #[test]
    fn recovery_ignores_server_file_names() -> TestResult {
        let base = Arc::new(Mutex::new(String::new()));
        let b = base.clone();
        let evil = Server::http(move |r| {
            let u = b.lock().map(|s| s.clone()).unwrap_or_default();
            match r.path.as_str() {
                "/InstallationPayload/RecoveryImage" => Reply::ok(
                    format!("AP: x\nAU: {u}/dl/%2e%2e\nAH: x\nAT: t\nCU: {u}/dl/%2e%2e%2f%2e%2e%2fvictim\nCH: x\nCT: t\n").into_bytes(),
                ),
                "/" => Reply::new(200, vec![("Set-Cookie".into(), "session=abc".into())], b"payload".to_vec()),
                _ => Reply::ok(b"payload".to_vec()),
            }
        })?;
        *base.lock().unwrap_or_else(|e| e.into_inner()) = evil.base.clone();
        let parent = tempdir()?;
        let dir = parent.join("staging");
        let http = Http::for_tests(&[], &evil.base);
        let (dmg, cnk) =
            download_recovery(&http, "sonoma", &dir, &AtomicBool::new(false), &mut |_| {})?;
        assert_eq!(
            (dmg, cnk),
            (dir.join("BaseSystem.dmg"), dir.join("BaseSystem.chunklist"))
        );
        assert_eq!(
            entries(&parent)?,
            vec!["staging".to_owned()],
            "nothing may be written outside the staging dir"
        );
        Ok(())
    }

    #[test]
    fn convert_progress() {
        let data = "    (0.00/100%)\r    (42.50/100%)\r    (100.00/100%)\n";
        let got: Vec<f64> = data
            .split(['\r', '\n'])
            .filter_map(|l| parse_convert_progress(l.trim()))
            .collect();
        assert_eq!(got, [0.0, 42.5, 100.0]);
    }

    #[test]
    fn download_failures_preserve_media() -> TestResult {
        if Command::new("qemu-img").arg("--version").output().is_err() {
            eprintln!("qemu-img missing: skipped");
            return Ok(());
        }
        let key = test_key()?;
        let ws = tempdir()?;
        fs::write(ws.join(MEDIA_FILE), "previous-media")?;
        fs::write(ws.join(MEDIA_MARKER), "monterey\n")?;
        let img = b"not-a-dmg".repeat(500);
        let f = fake_recovery()?;
        let never = AtomicBool::new(false);
        let run = || {
            download_media_with(
                &f.http,
                &ws,
                "sonoma",
                &key.to_public_key(),
                &never,
                &mut |_| {},
                &mut |_| {},
            )
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default()
        };

        // 1) tampered image: fails verification, nothing converted.
        {
            let mut s = f.st();
            s.chunklist = build_chunklist(&key, &img, 1024);
            s.dmg = [b"X".as_slice(), &img[1..]].concat();
        }
        let e = run();
        assert!(
            e.contains("verification"),
            "expected verification failure: {e}"
        );
        // 2) intact image that is not a DMG: fails conversion.
        f.st().dmg = img.clone();
        let e = run();
        assert!(e.contains("conversion"), "expected conversion failure: {e}");
        assert_eq!(
            fs::read_to_string(ws.join(MEDIA_FILE))?,
            "previous-media",
            "the previous media must stay"
        );
        assert_eq!(
            rg_core::Workspace::new(&ws).media_version(),
            "monterey",
            "the previous marker must stay"
        );
        assert!(!ws.join(STAGING_DIR).exists(), "staging must be removed");
        Ok(())
    }
}

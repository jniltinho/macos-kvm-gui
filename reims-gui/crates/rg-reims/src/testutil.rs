//! Test fixtures: temp Reims folders, tar.gz builders, a fake package and a local HTTPS server.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use tar::EntryType;
pub use tempfile::TempDir;

use crate::Reims;

/// Held by tests that run fake QEMUs: pruning reads the real /proc, where an exiting
/// `qemu-system-*` of another test would keep a build.
pub static QEMU_PROCS: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A Reims in `<t>/ws/.reims` with `<t>/os-release`, no `reims_qemu` setting and no upstream
/// install.
pub fn reims_in(t: &TempDir, os_release: &str) -> io::Result<Reims> {
    let mut r = Reims::new(t.path().join("ws/.reims"));
    r.os_release = t.path().join("os-release");
    write(&r.os_release, os_release)?;
    r.qemu_setting = None;
    r.data_dir = None;
    Ok(r)
}

/// Writes an executable file, creating its folder.
pub fn write(p: &Path, body: &str) -> io::Result<()> {
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(p, body)?;
    fs::set_permissions(p, fs::Permissions::from_mode(0o755))
}

pub struct Entry {
    name: String,
    typ: EntryType,
    link: String,
    body: String,
}

impl Entry {
    pub fn new(name: &str, typ: EntryType, link: &str, body: &str) -> Self {
        Self {
            name: name.into(),
            typ,
            link: link.into(),
            body: body.into(),
        }
    }
}

/// A .tar.gz with the names and links written raw (the tar crate's setters refuse `..`).
pub fn tar_gz(entries: &[Entry]) -> io::Result<Vec<u8>> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut b = tar::Builder::new(gz);
    for e in entries {
        let mut h = tar::Header::new_old();
        h.as_old_mut().name[..e.name.len()].copy_from_slice(e.name.as_bytes());
        h.as_old_mut().linkname[..e.link.len()].copy_from_slice(e.link.as_bytes());
        h.set_entry_type(e.typ);
        h.set_mode(0o755);
        let body = if e.typ == EntryType::Regular {
            e.body.as_bytes()
        } else {
            &[]
        };
        h.set_size(body.len() as u64);
        h.set_cksum();
        b.append(&h, body)?;
    }
    b.into_inner()?.finish()
}

/// A package whose QEMU is a script that lists apple-gfx-pci.
pub fn fake_package() -> io::Result<Vec<u8>> {
    tar_gz(&[
        Entry::new(
            "bin/qemu-system-x86_64",
            EntryType::Regular,
            "",
            "#!/bin/sh\necho 'name \"apple-gfx-pci\", bus PCI'\n",
        ),
        Entry::new("share/qemu/ReimsVgpu.rom", EntryType::Regular, "", "rom"),
    ])
}

pub struct Server {
    /// `https://127.0.0.1:<port>`.
    pub url: String,
    /// The server certificate (DER), to trust as a root.
    pub root: Vec<u8>,
}

/// Serves path -> body over TLS on 127.0.0.1 until the test process ends.
pub fn https_server(files: HashMap<String, Vec<u8>>) -> Result<Server, Box<dyn std::error::Error>> {
    let ck = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()])?;
    let root = ck.cert.der().to_vec();
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(ck.signing_key.serialize_der());
    let cfg = Arc::new(
        rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![ck.cert.der().clone()], key.into())?,
    );
    let l = TcpListener::bind("127.0.0.1:0")?;
    let url = format!("https://{}", l.local_addr()?);
    std::thread::spawn(move || {
        for tcp in l.incoming().flatten() {
            let Ok(conn) = rustls::ServerConnection::new(cfg.clone()) else {
                continue;
            };
            let mut s = rustls::StreamOwned::new(conn, tcp);
            let _ = serve(&mut s, &files);
        }
    });
    Ok(Server { url, root })
}

fn serve(s: &mut (impl io::Read + Write), files: &HashMap<String, Vec<u8>>) -> io::Result<()> {
    let mut r = BufReader::new(&mut *s);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let path = line.split(' ').nth(1).unwrap_or_default().to_string();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
    }
    let (status, body) = match files.get(&path) {
        Some(b) => ("200 OK", b.as_slice()),
        None => ("404 Not Found", &b"not found"[..]),
    };
    write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    s.write_all(body)?;
    s.flush()
}

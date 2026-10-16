//! Small helpers the Go code takes from other files of `internal/core`. rg-core is being written
//! in parallel; once it exposes these (`versionDisplay`, `SMBIOSModel`, `writeFileAtomic`,
//! `freeBytes`, `bridgeNameRe`), this module should re-export them instead.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// `config.go` Versions: short name and display name.
const VERSIONS: [(&str, &str); 9] = [
    ("high-sierra", "High Sierra (10.13)"),
    ("mojave", "Mojave (10.14)"),
    ("catalina", "Catalina (10.15)"),
    ("big-sur", "Big Sur (11)"),
    ("monterey", "Monterey (12)"),
    ("ventura", "Ventura (13)"),
    ("sonoma", "Sonoma (14)"),
    ("sequoia", "Sequoia (15)"),
    ("tahoe", "Tahoe (26)"),
];

/// `vms.go` versionDisplay: "Sonoma" for "sonoma".
pub(crate) fn version_display(short: &str) -> &str {
    VERSIONS
        .iter()
        .find(|(s, _)| *s == short)
        .map_or(short, |(_, name)| name.split(" (").next().unwrap_or(name))
}

/// `boot.go` SMBIOSModel: the Mac model the VM presents for a macOS version.
pub(crate) fn smbios_model(version: &str) -> &'static str {
    match version {
        "high-sierra" => "iMac18,3",
        "mojave" => "iMac19,1",
        _ => "iMac20,1",
    }
}

/// `network.go` bridgeNameRe `^[A-Za-z0-9_.-]{1,15}$`, used for build names too.
pub(crate) fn valid_name(s: &str) -> bool {
    (1..=15).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// `config.go` writeFileAtomic: a temporary file in the same folder, renamed over `path`.
pub(crate) fn write_file_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let tmp = dir.join(format!(".tmp-{name}-{}", std::process::id()));
    let res = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

/// `workspace.go` freeBytes: bytes available to this user on the file system of `dir`.
pub(crate) fn free_bytes(dir: &Path) -> io::Result<u64> {
    let st = rustix::fs::statvfs(dir)?;
    Ok(st.f_bavail.saturating_mul(st.f_frsize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_display_strips_the_number() {
        assert_eq!(version_display("big-sur"), "Big Sur");
        assert_eq!(version_display("leopard"), "leopard");
    }

    #[test]
    fn valid_name_matches_the_go_regex() {
        assert!(valid_name("24H32") && valid_name("a.b-c_d"));
        assert!(!valid_name("") && !valid_name("../x/y") && !valid_name("0123456789abcdef"));
    }
}

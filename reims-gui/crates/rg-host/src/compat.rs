//! Pieces of the Go `internal/core` that rg-core (or rg-assets/rg-reims) will own but does not
//! have yet. Each is the smallest port this crate needs; when the owning crate gains the real
//! one, call it and delete the copy here.

use std::path::PathBuf;

/// The macOS versions in release order (Go `Versions`): the index decides the hardware profile.
const VERSIONS: [(&str, &str); 9] = [
    ("high-sierra", "High Sierra"),
    ("mojave", "Mojave"),
    ("catalina", "Catalina"),
    ("big-sur", "Big Sur"),
    ("monterey", "Monterey"),
    ("ventura", "Ventura"),
    ("sonoma", "Sonoma"),
    ("sequoia", "Sequoia"),
    ("tahoe", "Tahoe"),
];

const IDX_VENTURA: usize = 5;

/// Go `versionIndex`; `None` (Go's -1) sorts before every known version.
pub(crate) fn version_index(short: &str) -> Option<usize> {
    VERSIONS.iter().position(|(s, _)| *s == short)
}

/// Go `versionDisplay`: "Big Sur" for "big-sur", the short name when unknown.
pub(crate) fn version_display(short: &str) -> &str {
    VERSIONS
        .iter()
        .find(|(s, _)| *s == short)
        .map_or(short, |(_, n)| n)
}

/// Go `ProfileFor(version).NeedsAVX2`: Ventura and later need SSE4.1 and AVX2.
pub(crate) fn needs_avx2(version: &str) -> bool {
    version_index(version).is_some_and(|i| i >= IDX_VENTURA)
}

/// Go `RootPatchVersion`: root-patch mode applies from Monterey to Sequoia.
pub(crate) fn root_patch_version(version: &str) -> bool {
    matches!(version, "monterey" | "ventura" | "sonoma" | "sequoia")
}

/// Go `SettingsFile`: `$XDG_CONFIG_HOME` (or `~/.config`)`/macos-kvm-gui/settings.yaml`.
pub(crate) fn settings_file() -> String {
    config_dir().map_or_else(String::new, |d| {
        d.join("macos-kvm-gui/settings.yaml")
            .to_string_lossy()
            .into_owned()
    })
}

fn config_dir() -> Option<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(p) if p.is_absolute() => Some(p),
        _ => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")),
    }
}

/// Part of Go `ReimsQEMU`: `MKG_REIMS_QEMU`, then an upstream reims-ctl install.
// ponytail: skips settings.yaml's reims_qemu and the app's own <Workspace>/.reims install
// (rg-core/rg-reims own those); the app sets `Host::reims_qemu` to the full lookup.
pub(crate) fn reims_qemu() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("MKG_REIMS_QEMU").filter(|p| !p.is_empty()) {
        let p = std::path::absolute(PathBuf::from(p)).map_err(|e| e.to_string())?;
        return std::fs::metadata(&p)
            .map(|_| p)
            .map_err(|e| format!("reims_qemu in {}: {e}", settings_file()));
    }
    let data = match std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        Some(p) if p.is_absolute() => p,
        _ => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"),
    };
    let p = data.join("reims-ctl/qemu/current/bin/qemu-system-x86_64");
    if p.is_file() {
        return Ok(p);
    }
    Err(format!(
        "Reims vGPU is not installed (looked for {}): install it in the Host tab or with `macos-kvm-gui reims install`",
        p.display()
    ))
}

/// Go `LoadExtraKexts`, reduced to what eligibility needs: how many kexts settings.yaml lists.
// ponytail: always 0 until rg-assets ports settings.yaml's kexts; the app sets
// `Host::extra_kexts` to the real loader, so a patch-kext GPU is refused until then.
pub(crate) fn extra_kexts() -> Result<usize, String> {
    Ok(0)
}

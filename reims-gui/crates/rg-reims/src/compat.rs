//! Stand-ins for rg-core paths and settings that are being ported in parallel.
//!
//! ponytail: replace with rg-core's `WorkspaceDir`, `SettingsFile` and `Setting` once they land;
//! the `reims_qemu` lookup here scans settings.yaml line by line instead of parsing YAML.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// `<Workspace>/.reims` for a Workspace root (`~` and `~/...` expanded; it must be absolute).
pub fn reims_dir(workspace_root: &str) -> Option<PathBuf> {
    let p = match workspace_root.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            dirs::home_dir()?.join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(workspace_root),
    };
    p.is_absolute().then(|| p.join(".reims"))
}

/// `~/.config/macos-kvm-gui/settings.yaml`.
pub fn settings_file() -> Option<PathBuf> {
    Some(
        dirs::config_dir()?
            .join("macos-kvm-gui")
            .join("settings.yaml"),
    )
}

/// The Go app's `Setting("MKG_REIMS_QEMU")` made absolute: the environment (relative to the
/// current directory), then `reims_qemu` in settings.yaml or the older `.env` file (relative to
/// their folder). `None` when unset or empty.
pub fn reims_qemu_setting() -> Option<PathBuf> {
    let (value, base) = match env::var("MKG_REIMS_QEMU") {
        Ok(v) => (v, None),
        Err(_) => {
            let file = settings_file()?;
            let dir = file.parent()?.to_path_buf();
            let v = fs::read_to_string(&file)
                .ok()
                .and_then(|s| yaml_value(&s, "reims_qemu"))
                .or_else(|| {
                    let env = fs::read_to_string(dir.join(".env")).ok()?;
                    env_value(&env, "MKG_REIMS_QEMU")
                })?;
            (v, Some(dir))
        }
    };
    if value.is_empty() {
        return None;
    }
    let p = match base {
        Some(dir) if !Path::new(&value).is_absolute() => dir.join(value),
        _ => PathBuf::from(value),
    };
    std::path::absolute(&p).ok().or(Some(p))
}

/// A top-level `key: value` of a YAML file, unquoted, without a trailing comment.
fn yaml_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let v = line.strip_prefix(key)?.strip_prefix(':')?.trim();
        let v = match v.chars().next() {
            Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or_default(),
            _ => v.split(" #").next().unwrap_or_default().trim(),
        };
        Some(v.to_string())
    })
}

/// A `KEY=value` of a `.env` file (`#` comments, quotes dropped).
fn env_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (k, v) = line.trim().split_once('=')?;
        (!k.starts_with('#') && k.trim() == key)
            .then(|| v.trim().trim_matches(['"', '\'']).to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_value_reads_plain_quoted_and_commented_values() {
        let y = "# reims_qemu: /no\ntheme: dark\nreims_qemu: \"/opt/q # x\"\n";
        assert_eq!(yaml_value(y, "reims_qemu").as_deref(), Some("/opt/q # x"));
        assert_eq!(
            yaml_value("reims_qemu: q/bin # here\n", "reims_qemu").as_deref(),
            Some("q/bin")
        );
        assert_eq!(yaml_value("  reims_qemu: nested\n", "reims_qemu"), None);
    }

    #[test]
    fn env_value_skips_comments() {
        let e = "#MKG_REIMS_QEMU=/no\nMKG_REIMS_QEMU = '/yes'\n";
        assert_eq!(env_value(e, "MKG_REIMS_QEMU").as_deref(), Some("/yes"));
    }

    #[test]
    fn reims_dir_needs_an_absolute_workspace() {
        assert_eq!(reims_dir("/vms"), Some(PathBuf::from("/vms/.reims")));
        assert_eq!(reims_dir("vms"), None);
        assert!(reims_dir("~/macOS-VMs").is_some_and(|p| p.ends_with("macOS-VMs/.reims")));
    }
}

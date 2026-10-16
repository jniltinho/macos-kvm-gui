//! Types this crate needs that belong to `rg-core` (settings.yaml). They move there at
//! integration, where settings.yaml parsing calls [`ExtraKext::check`] on every entry; this
//! crate then re-exports `rg-core`'s types instead.

use rg_core::Config;

/// One `kexts:` entry of settings.yaml (Go `core.ExtraKext`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtraKext {
    /// An https:// URL of a zip archive.
    pub url: String,
    /// Lowercase hex SHA-256 of the archive; empty = not checked.
    pub sha256: String,
    /// The kexts to load, in order (`load:`); `None` = the archive's only kext.
    pub kexts: Option<Vec<String>>,
}

/// The kexts part of settings.yaml (Go `core.ExtraKexts`); a start reads it once and uses
/// that snapshot for every step.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtraKexts {
    /// Built-in kexts whose entry is disabled (`disable:`).
    pub disabled: Vec<String>,
    /// `kexts:`.
    pub entries: Vec<ExtraKext>,
}

impl ExtraKext {
    /// Validates and normalizes an entry read from settings.yaml: an https:// URL, an optional
    /// SHA-256 (lowercased), valid kext names (`load: []` becomes `None`).
    pub fn check(&mut self) -> crate::Result<()> {
        let url = &self.url;
        if !url.starts_with("https://")
            || url.len() == "https://".len()
            || url.contains([' ', '\t', '\n'])
        {
            return crate::invalid("url must be an https:// URL");
        }
        self.sha256 = self.sha256.to_lowercase();
        if !self.sha256.is_empty() && !crate::extrakexts::valid_sha256(&self.sha256) {
            return crate::invalid("sha256 must be 64 hex digits");
        }
        if self.kexts.as_ref().is_some_and(Vec::is_empty) {
            self.kexts = None;
        }
        for n in self.kexts.iter().flatten() {
            if !crate::extrakexts::valid_kext_name(n) {
                return crate::invalid(format!("invalid kext name {n:?} in load"));
            }
        }
        Ok(())
    }
}

/// Go `Config.ReimsKext()`: Reims vGPU on Tahoe, the only version ReimsVgpu.kext loads on.
pub(crate) fn reims_kext(c: &Config) -> bool {
    c.reims() && c.version == "tahoe"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(url: &str, sha: &str, load: Option<&[&str]>) -> ExtraKext {
        ExtraKext {
            url: url.into(),
            sha256: sha.into(),
            kexts: load.map(|l| l.iter().map(|s| s.to_string()).collect()),
        }
    }

    #[test]
    fn check_normalizes_valid_entries() {
        let mut e = entry("https://example.com/a.zip", &"AB".repeat(32), Some(&[]));
        assert!(e.check().is_ok());
        assert_eq!(e.sha256, "ab".repeat(32));
        assert_eq!(e.kexts, None, "load: [] is no load list");
        let mut e = entry("https://example.com/b.zip", "", Some(&["B", "A"]));
        assert!(e.check().is_ok());
    }

    #[test]
    fn check_refuses_invalid_entries() {
        for (mut e, want) in [
            (entry("http://example.com/a.zip", "", None), "https://"),
            (entry("https://", "", None), "https://"),
            (entry("https://example.com/a b.zip", "", None), "https://"),
            (entry("https://example.com/a.zip", "abc", None), "64 hex"),
            (
                entry("https://example.com/a.zip", "", Some(&["a/b"])),
                "invalid kext name",
            ),
        ] {
            let err = e.check().err().map(|e| e.to_string()).unwrap_or_default();
            assert!(err.contains(want), "{e:?}: want {want:?}, got {err:?}");
        }
    }
}

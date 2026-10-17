//! Root-patch mode (experimental, per VM, off by default; Go `rootpatch.go`): OpenCore
//! settings that let OpenCore Legacy Patcher (OCLP) root-patch a passed-through NVIDIA Kepler
//! GPU inside Monterey to Sequoia, matching OCLP 2.5.1's own profile for Kepler: SIP lowered
//! to 0x803, AMFIPass instead of disabling AMFI, RSRHelper with -nokcmismatchpanic,
//! ipc_control_port_options=0, KDKlessWorkaround, and RestrictEvents blocking mediaanalysisd
//! (revblock=media). The app never runs OCLP: the user root-patches inside macOS.

use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use rg_core::Config;
use rg_core::workspace::ROOT_PATCH_MARKER;

use crate::boot::{KextCache, copy_kext};
use crate::extrakexts::MAX_KEXT_FILE;
use crate::http::https_get_max;
use crate::util::{TempDir, exists, sha256_hex};
use crate::{Context, Result, invalid};

/// OCLP's 2.5.1 tag. The kexts are downloaded from that commit, never embedded: AMFIPass,
/// RSRHelper and KDKlessWorkaround are published only as binaries inside OCLP.
const OCLP_COMMIT: &str = "f40057a5292f4804b51bcfe78d5047c7302a6434";

#[derive(Clone, Debug)]
pub(crate) struct OclpKext {
    pub(crate) name: &'static str,
    zip: &'static str,
    sha256: String,
    min_kernel: &'static str,
}

/// Loaded in this order, after Lilu (RestrictEvents and AMFIPass are Lilu plugins). MinKernel
/// follows OCLP's config.plist.
pub(crate) fn root_patch_kexts() -> [OclpKext; 4] {
    let k = |name, zip, sha256: &str, min_kernel| OclpKext {
        name,
        zip,
        sha256: sha256.into(),
        min_kernel,
    };
    [
        k(
            "AMFIPass",
            "Acidanthera/AMFIPass-v1.4.1-RELEASE.zip",
            "07b266145906db41f4b13a7938fbb173ea28888cc1fa65f84417f8820adc961e",
            "20.0.0",
        ),
        k(
            "RSRHelper",
            "Acidanthera/RSRHelper-v1.0.2-RELEASE.zip",
            "7fa418a7aa35847826a2b7b603987495d5e79644a139866d447e2489b020e623",
            "20.0.0",
        ),
        k(
            "RestrictEvents",
            "Acidanthera/RestrictEvents-v1.1.7-RELEASE.zip",
            "d11e278544a9f1104df60770d2cf2cc2573e8ecaee1dd6d291f4b552975af286",
            "12.0.0",
        ),
        k(
            "KDKlessWorkaround",
            "Misc/KDKlessWorkaround-v1.0.0-RELEASE.zip",
            "8bdb85181d587cf8663e07f2853bc3e6ffa4712300b0dbcf954a97a25da6d0f1",
            "22.3.0",
        ),
    ]
}

fn kext_ready(dir: &Path, name: &str) -> bool {
    exists(&dir.join(format!("{name}.kext/Contents/MacOS/{name}")))
}

/// Go `RootPatchKextsReady`: every OCLP kext is in the cache.
pub fn root_patch_kexts_ready(cache: &KextCache) -> bool {
    let dir = cache.root_patch_dir();
    root_patch_kexts().iter().all(|k| kext_ready(&dir, k.name))
}

/// Go `DownloadRootPatchKexts`: fetches the kexts from OCLP's pinned commit, checks each
/// archive's SHA-256 and unpacks only its .kext into the cache.
pub fn download_root_patch_kexts(cache: &KextCache, cancel: &AtomicBool) -> Result<()> {
    let base = format!(
        "https://raw.githubusercontent.com/dortania/OpenCore-Legacy-Patcher/{OCLP_COMMIT}/payloads/Kexts/"
    );
    download_from(&base, &root_patch_kexts(), cache, cancel)
}

fn download_from(
    base: &str,
    kexts: &[OclpKext],
    cache: &KextCache,
    cancel: &AtomicBool,
) -> Result<()> {
    let dir = cache.root_patch_dir();
    fs::create_dir_all(&dir)?;
    for k in kexts.iter().filter(|k| !kext_ready(&dir, k.name)) {
        let data = https_get_max(&format!("{base}{}", k.zip), 16 << 20, cancel)
            .context(|| k.name.into())?;
        let sum = sha256_hex(&data);
        if sum != k.sha256 {
            return invalid(format!(
                "{}: the download does not match OCLP 2.5.1's file (SHA-256 {sum})",
                k.name
            ));
        }
        unpack_kext(&data, k.name, &dir).context(|| k.name.into())?;
    }
    Ok(())
}

/// Go `unpackKext`: extracts `<name>.kext` from a zip into `dir`, through a temporary folder so
/// a partial kext never stays. Entries outside `<name>.kext/` (such as __MACOSX) are skipped,
/// and none may leave it.
pub(crate) fn unpack_kext(data: &[u8], name: &str, dir: &Path) -> Result<()> {
    let mut zr = zip::ZipArchive::new(Cursor::new(data))?;
    let tmp = TempDir::new_in(dir, &format!(".{name}-"))?;
    let prefix = format!("{name}.kext/");
    for i in 0..zr.len() {
        let mut f = zr.by_index(i)?;
        let fname = f.name().to_string();
        let Some(rest) = fname.strip_prefix(&prefix) else {
            continue;
        };
        if f.is_dir() {
            continue;
        }
        // filepath.Join cleans "..": refuse any entry that is not plainly inside the kext.
        if Path::new(rest)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
            || rest.is_empty()
        {
            return invalid(format!("unexpected path {fname:?} in the archive"));
        }
        if f.is_symlink() {
            return invalid(format!("unexpected link {fname:?} in the archive"));
        }
        let size = f.size();
        if size > MAX_KEXT_FILE {
            return invalid(format!(
                "{fname} is larger than {} MiB",
                MAX_KEXT_FILE >> 20
            ));
        }
        let dst = tmp.path().join(&fname);
        if let Some(p) = dst.parent() {
            fs::create_dir_all(p)?;
        }
        // One byte more than the header says: a file is never cut short silently.
        let mut buf = Vec::new();
        (&mut f).take(size + 1).read_to_end(&mut buf)?;
        if buf.len() as u64 != size {
            return invalid(format!("{fname} does not match its size in the archive"));
        }
        fs::write(&dst, buf)?;
    }
    if !kext_ready(tmp.path(), name) {
        return invalid(format!("the archive has no {name}.kext"));
    }
    let final_ = dir.join(format!("{name}.kext"));
    let _ = fs::remove_dir_all(&final_);
    fs::rename(tmp.path().join(format!("{name}.kext")), final_)?;
    Ok(())
}

/// Go `addRootPatchKexts`: copies the cached OCLP kexts into the extracted EFI.
pub(crate) fn add_root_patch_kexts(cache: &KextCache, kexts_dir: &Path) -> Result<()> {
    if !root_patch_kexts_ready(cache) {
        return invalid(
            "root-patch mode needs OCLP's kexts: download them in the GPU tab (or config set root-patch=true)",
        );
    }
    for k in root_patch_kexts() {
        copy_kext(
            &cache.root_patch_dir().join(format!("{}.kext", k.name)),
            kexts_dir,
            k.name,
        )?;
    }
    Ok(())
}

/// The boot arguments OCLP adds with lowered SIP.
pub(crate) const ROOT_PATCH_BOOT_ARGS: &str = " ipc_control_port_options=0 -nokcmismatchpanic";

/// The config.plist text for the mode (Go `rootPatchPlist`'s map).
pub(crate) struct RootPatchPlist {
    pub(crate) csr: &'static str,
    pub(crate) kexts: String,
    pub(crate) csr_delete: &'static str,
    pub(crate) revblock_add: &'static str,
    pub(crate) revblock_delete: &'static str,
}

/// On adds the kexts, the lowered SIP and revblock; `clear` only deletes the values a previous
/// "on" boot left in NVRAM.
pub(crate) fn root_patch_plist(on: bool, clear: bool) -> RootPatchPlist {
    let mut r = RootPatchPlist {
        csr: "AAAAAA==",
        kexts: String::new(),
        csr_delete: "",
        revblock_add: "",
        revblock_delete: "",
    };
    if on || clear {
        r.csr_delete = "\n\t\t\t\t<string>csr-active-config</string>";
        r.revblock_delete = "\n\t\t\t\t<string>revblock</string>";
    }
    if !on {
        return r;
    }
    r.csr = "AwgAAA=="; // 0x803 little-endian: OCLP's lowered SIP
    r.revblock_add = "\n\t\t\t\t<key>revblock</key>\n\t\t\t\t<string>media</string>";
    for k in root_patch_kexts() {
        r.kexts += &kext_entry(
            k.name,
            k.min_kernel,
            &format!("{}.kext (root-patch mode, from OCLP 2.5.1)", k.name),
        );
    }
    r
}

/// One Kernel > Add entry; name and comment need no XML escaping.
pub(crate) fn kext_entry(name: &str, min_kernel: &str, comment: &str) -> String {
    format!(
        "\n\t\t\t<dict>\n\t\t\t\t<key>Arch</key>\n\t\t\t\t<string>x86_64</string>\n\t\t\t\t<key>BundlePath</key>\n\t\t\t\t<string>{name}.kext</string>\
         \n\t\t\t\t<key>Comment</key>\n\t\t\t\t<string>{comment}</string>\n\t\t\t\t<key>Enabled</key>\n\t\t\t\t<true/>\
         \n\t\t\t\t<key>ExecutablePath</key>\n\t\t\t\t<string>Contents/MacOS/{name}</string>\n\t\t\t\t<key>MaxKernel</key>\n\t\t\t\t<string></string>\
         \n\t\t\t\t<key>MinKernel</key>\n\t\t\t\t<string>{min_kernel}</string>\n\t\t\t\t<key>PlistPath</key>\n\t\t\t\t<string>Contents/Info.plist</string>\n\t\t\t</dict>"
    )
}

/// Go `ClearRootPatchMarker`: ends the SIP restoration after a VM with the mode off ran long
/// enough for OpenCore to have applied its NVRAM Delete (it does so within seconds).
pub fn clear_root_patch_marker(ws_dir: &Path, c: &Config, ran: Duration) {
    if !c.root_patch && ran >= Duration::from_secs(60) {
        let _ = fs::remove_file(ws_dir.join(ROOT_PATCH_MARKER));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot::{BootOptions, BootState, Identity, opencore_config, prepare_boot};
    use crate::util::testutil::{serve, tempdir, zip_of};
    use crate::{BootAssets, Error};
    use rg_core::workspace::EFI_DIR;

    #[test]
    fn root_patch_config_on_off_and_clear() -> Result<()> {
        let id = Identity::generate()?;
        let mut c = Config::defaults();
        c.version = "sequoia".into();
        let o = BootOptions {
            keyboard: "en-US:0",
            ..Default::default()
        };
        let a = BootAssets::Embedded;
        let off = opencore_config(&a, &c, &id, &o, &BootState::default())?;
        c.root_patch = true;
        let on = opencore_config(&a, &c, &id, &o, &BootState::default())?;
        c.root_patch = false;
        let clear = opencore_config(
            &a,
            &c,
            &id,
            &o,
            &BootState {
                root_patch_was_on: true,
                ..Default::default()
            },
        )?;

        // Never turned on: full SIP, no extra kexts, the user's csr-active-config kept.
        for s in [
            "AMFIPass",
            "revblock",
            "<string>csr-active-config</string>",
            "ipc_control_port_options",
            "AwgAAA==",
        ] {
            assert!(!off.contains(s), "default config contains {s:?}");
        }
        assert!(off.contains("<key>csr-active-config</key>\n\t\t\t\t<data>\n\t\t\t\tAAAAAA=="));
        for s in [
            "AwgAAA==",
            "keepsyms=1 ipc_control_port_options=0 -nokcmismatchpanic",
            "<key>revblock</key>\n\t\t\t\t<string>media</string>",
            "<string>csr-active-config</string>",
            "<string>revblock</string>",
        ] {
            assert!(on.contains(s), "root-patch config lacks {s:?}");
        }
        let mut last = on
            .find("<string>WhateverGreen.kext</string>")
            .unwrap_or(usize::MAX);
        for k in root_patch_kexts() {
            let i = on
                .find(&format!("<string>{}.kext</string>", k.name))
                .unwrap_or(0);
            assert!(
                i > last && on[i..].contains(&format!("<string>{}</string>", k.min_kernel)),
                "{} order",
                k.name
            );
            last = i;
        }
        assert!(!on.contains("amfi=") && !on.contains("amfi_get_out_of_my_way"));
        assert!(
            !clear.contains("AMFIPass")
                && !clear.contains("AwgAAA==")
                && !clear.contains("<key>revblock</key>")
        );
        assert!(
            clear.contains("<string>csr-active-config</string>")
                && clear.contains("<string>revblock</string>")
        );
        Ok(())
    }

    #[test]
    fn root_patch_entry_unchanged() {
        let want = "\n\t\t\t<dict>\n\t\t\t\t<key>Arch</key>\n\t\t\t\t<string>x86_64</string>\n\t\t\t\t<key>BundlePath</key>\n\t\t\t\t<string>AMFIPass.kext</string>\
            \n\t\t\t\t<key>Comment</key>\n\t\t\t\t<string>AMFIPass.kext (root-patch mode, from OCLP 2.5.1)</string>\n\t\t\t\t<key>Enabled</key>\n\t\t\t\t<true/>\
            \n\t\t\t\t<key>ExecutablePath</key>\n\t\t\t\t<string>Contents/MacOS/AMFIPass</string>\n\t\t\t\t<key>MaxKernel</key>\n\t\t\t\t<string></string>\
            \n\t\t\t\t<key>MinKernel</key>\n\t\t\t\t<string>20.0.0</string>\n\t\t\t\t<key>PlistPath</key>\n\t\t\t\t<string>Contents/Info.plist</string>\n\t\t\t</dict>";
        assert!(root_patch_plist(true, false).kexts.starts_with(want));
    }

    /// An archive like OCLP's: the kext, macOS metadata and, optionally, a hostile entry.
    fn kext_zip(name: &str, extra: &[&str]) -> Vec<u8> {
        let files = [
            format!("{name}.kext/Contents/Info.plist"),
            format!("{name}.kext/Contents/MacOS/{name}"),
            format!("__MACOSX/{name}.kext/._x"),
        ];
        let mut all: Vec<&str> = files.iter().map(String::as_str).collect();
        all.extend_from_slice(extra);
        zip_of(&all)
    }

    #[test]
    fn unpack_refuses_escapes_and_size_mismatches() -> Result<()> {
        let t = tempdir();
        assert!(
            unpack_kext(
                &kext_zip("Evil", &["Evil.kext/../../escape"]),
                "Evil",
                t.path()
            )
            .is_err()
        );
        // Go-built: stored, 5 bytes of data, a header that says 3.
        let mismatch = include_bytes!("../tests/fixtures/size-mismatch.zip");
        assert!(unpack_kext(mismatch, "Foo", t.path()).is_err());
        unpack_kext(&kext_zip("Good", &[]), "Good", t.path())?;
        assert!(kext_ready(t.path(), "Good") && !exists(&t.path().join("__MACOSX")));
        assert_eq!(
            fs::read_dir(t.path())?.count(),
            1,
            "no temporary folder left"
        );
        Ok(())
    }

    #[test]
    fn root_patch_kexts_download_and_transitions() -> Result<()> {
        let root = tempdir();
        let cache = KextCache::new(root.path());
        let mut kexts = root_patch_kexts();
        let mut files = std::collections::HashMap::new();
        for k in &mut kexts {
            let data = kext_zip(k.name, &[]);
            k.sha256 = sha256_hex(&data);
            files.insert(format!("/{}", k.zip), data);
        }
        let (base, _) = serve(files);
        let base = format!("{base}/");
        let no = AtomicBool::new(false);
        let good = kexts[0].sha256.clone();
        kexts[0].sha256 = "0".repeat(64);
        assert!(
            download_from(&base, &kexts, &cache, &no).is_err() && !root_patch_kexts_ready(&cache)
        );
        kexts[0].sha256 = good;
        download_from(&base, &kexts, &cache, &no)?;
        assert!(
            root_patch_kexts_ready(&cache) && !exists(&cache.root_patch_dir().join("__MACOSX"))
        );

        // A VM: on copies the kexts and leaves the marker; the next boot with it off deletes the
        // lowered SIP once; the one after is the default again.
        let ws = root.path().join("vm");
        let mut c = Config::defaults();
        c.version = "sequoia".into();
        c.root_patch = true;
        let o = BootOptions {
            keyboard: "en-US:0",
            ..Default::default()
        };
        let read =
            || fs::read_to_string(ws.join(EFI_DIR).join("EFI/OC/config.plist")).unwrap_or_default();
        prepare_boot(&ws, &c, &BootAssets::Embedded, &cache, &o)?;
        assert!(exists(
            &ws.join(EFI_DIR)
                .join("EFI/OC/Kexts/AMFIPass.kext/Contents/MacOS/AMFIPass")
        ));
        assert!(exists(&ws.join(ROOT_PATCH_MARKER)) && read().contains("AwgAAA=="));
        c.root_patch = false;
        prepare_boot(&ws, &c, &BootAssets::Embedded, &cache, &o)?;
        assert!(read().contains("<string>csr-active-config</string>"));
        assert!(!exists(
            &ws.join(EFI_DIR).join("EFI/OC/Kexts/AMFIPass.kext")
        ));
        // Writing a config or a QEMU that stops at once does not end it.
        prepare_boot(&ws, &c, &BootAssets::Embedded, &cache, &o)?;
        clear_root_patch_marker(&ws, &c, Duration::from_secs(5));
        assert!(
            exists(&ws.join(ROOT_PATCH_MARKER))
                && read().contains("<string>csr-active-config</string>")
        );
        clear_root_patch_marker(&ws, &c, Duration::from_secs(120));
        prepare_boot(&ws, &c, &BootAssets::Embedded, &cache, &o)?;
        assert!(
            !read().contains("<string>csr-active-config</string>"),
            "later boots are the default again"
        );
        Ok(())
    }

    #[test]
    fn root_patch_without_cached_kexts_is_refused() {
        let root = tempdir();
        let mut c = Config::defaults();
        c.root_patch = true;
        let o = BootOptions {
            keyboard: "en-US:0",
            ..Default::default()
        };
        let err = prepare_boot(
            &root.path().join("vm"),
            &c,
            &BootAssets::Embedded,
            &KextCache::new(root.path()),
            &o,
        );
        assert!(matches!(err, Err(Error::Invalid(m)) if m.contains("OCLP")));
    }
}

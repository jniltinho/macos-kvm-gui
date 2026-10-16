//! ReimsVgpu.kext (steelbrain/reims-util, GPL-2.0-or-later OR LGPL-3.0-or-later, with Lilu code
//! under BSD-3-Clause) lets WindowServer drive Reims vGPU displays wider than 2048 px. It is a
//! Lilu plugin that loads on macOS 26 only, so Tahoe VMs on Reims vGPU get it. The boot argument
//! -reimsvgpuoff turns it off. It is downloaded from a pinned commit, never embedded (Go
//! `reimskext.go`).

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use crate::boot::{KextCache, copy_kext};
use crate::http::https_get_max;
use crate::util::{TempDir, exists, sha256_hex};
use crate::{Context, Result, invalid};

/// The reims-util commit the kext comes from.
pub const REIMS_UTIL_COMMIT: &str = "b302389a957c56fad8e5b3b3b16107932ea9e5e9";

/// The kext's files, relative to ReimsVgpu.kext, with their SHA-256.
const REIMS_KEXT_FILES: [(&str, &str); 2] = [
    (
        "Contents/Info.plist",
        "f572c4a3e5496110f0d6799297a1e215409d4df2e300c22e6070b97d5f3d9b66",
    ),
    (
        "Contents/MacOS/ReimsVgpu",
        "d028c738a707af0de69a78b455bbca8ea39e4d8b71a56c51f432bbf1c29915c3",
    ),
];

/// Darwin 25, macOS 26: the kext does nothing before it.
pub(crate) const REIMS_KEXT_MIN_KERNEL: &str = "25.0.0";

/// Go `ReimsKextReady`: the kext is in the cache.
pub fn reims_kext_ready(cache: &KextCache) -> bool {
    let dir = cache.reims_kext_dir();
    REIMS_KEXT_FILES.iter().all(|(p, _)| exists(&dir.join(p)))
}

/// Go `DownloadReimsKext`: fetches the kext's files from the pinned commit, checks their
/// SHA-256 and publishes the kext into the cache only when every file matches.
pub fn download_reims_kext(cache: &KextCache, cancel: &AtomicBool) -> Result<()> {
    let base = format!(
        "https://raw.githubusercontent.com/steelbrain/reims-util/{REIMS_UTIL_COMMIT}/kext/ReimsVgpu.kext/"
    );
    download_from(&base, &REIMS_KEXT_FILES, cache, cancel)
}

fn download_from(
    base: &str,
    files: &[(&str, &str)],
    cache: &KextCache,
    cancel: &AtomicBool,
) -> Result<()> {
    if reims_kext_ready(cache) {
        return Ok(());
    }
    let final_ = cache.reims_kext_dir();
    let parent = final_.parent().unwrap_or(Path::new("/"));
    fs::create_dir_all(parent)?;
    let tmp = TempDir::new_in(parent, ".ReimsVgpu-")?;
    for (p, want) in files {
        let data = https_get_max(&format!("{base}{p}"), 4 << 20, cancel)
            .context(|| "ReimsVgpu.kext".into())?;
        let sum = sha256_hex(&data);
        if sum != *want {
            return invalid(format!(
                "ReimsVgpu.kext: {p} does not match reims-util {} (SHA-256 {sum})",
                &REIMS_UTIL_COMMIT[..7]
            ));
        }
        let dst = tmp.path().join(p);
        if let Some(d) = dst.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(dst, data)?;
    }
    match fs::rename(tmp.path(), &final_) {
        Err(e) if !reims_kext_ready(cache) => Err(e.into()), // another process may have won
        _ => Ok(()),
    }
}

/// Go `addReimsKext`: copies the cached kext into the extracted EFI.
pub(crate) fn add_reims_kext(cache: &KextCache, kexts_dir: &Path) -> Result<()> {
    if !reims_kext_ready(cache) {
        return invalid(
            "Reims vGPU on Tahoe needs ReimsVgpu.kext: start the VM again to download it",
        );
    }
    copy_kext(&cache.reims_kext_dir(), kexts_dir, "ReimsVgpu")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BootAssets;
    use crate::boot::{BootOptions, BootState, Identity, opencore_config, prepare_boot};
    use crate::util::testutil::{serve, tempdir};
    use rg_core::Config;
    use rg_core::ExtraKexts;
    use rg_core::config::{GRAPHICS_REIMS, GRAPHICS_SVGA};
    use rg_core::workspace::EFI_DIR;

    #[test]
    fn reims_kext_download_and_boot() -> Result<()> {
        let root = tempdir();
        let cache = KextCache::new(root.path());
        let plist: &[u8] = b"<plist/>";
        let bin: &[u8] = b"binary";
        let (base, _) = serve(
            [
                ("/Contents/Info.plist".to_string(), plist.to_vec()),
                ("/Contents/MacOS/ReimsVgpu".to_string(), bin.to_vec()),
            ]
            .into(),
        );
        let base = format!("{base}/");
        let (sp, sb) = (sha256_hex(plist), sha256_hex(bin));
        let no = AtomicBool::new(false);
        let zero = "0".repeat(64);
        let dir = cache.reims_kext_dir();
        let bad = [
            (REIMS_KEXT_FILES[0].0, sp.as_str()),
            (REIMS_KEXT_FILES[1].0, zero.as_str()),
        ];
        assert!(download_from(&base, &bad, &cache, &no).is_err());
        assert!(
            !reims_kext_ready(&cache) && !exists(&dir),
            "a file that does not match caches nothing"
        );
        let left = fs::read_dir(dir.parent().unwrap_or(&dir))?.count();
        assert_eq!(left, 0, "temporary folders left");
        let good = [
            (REIMS_KEXT_FILES[0].0, sp.as_str()),
            (REIMS_KEXT_FILES[1].0, sb.as_str()),
        ];
        download_from(&base, &good, &cache, &no)?;
        assert!(reims_kext_ready(&cache));
        assert!(dir.ends_with(".kexts/reims-util-b302389/ReimsVgpu.kext"));

        let ws = root.path().join("vm");
        let read =
            || fs::read_to_string(ws.join(EFI_DIR).join("EFI/OC/config.plist")).unwrap_or_default();
        let kext = ws
            .join(EFI_DIR)
            .join("EFI/OC/Kexts/ReimsVgpu.kext/Contents/MacOS/ReimsVgpu");
        let empty = ExtraKexts::default();
        for (version, graphics, start, want) in [
            ("tahoe", GRAPHICS_REIMS, true, true),
            ("tahoe", GRAPHICS_REIMS, false, false), // creating the VM: offline, no kext
            ("sequoia", GRAPHICS_REIMS, true, false),
            ("tahoe", GRAPHICS_SVGA, true, false),
        ] {
            let mut c = Config::defaults();
            c.version = version.into();
            c.graphics = graphics.into();
            let o = BootOptions {
                extras: start.then_some(&empty),
                keyboard: "en-US:0",
                ..Default::default()
            };
            prepare_boot(&ws, &c, &BootAssets::Embedded, &cache, &o)?;
            let got = exists(&kext) && read().contains("<string>ReimsVgpu.kext</string>");
            assert_eq!(got, want, "{version} {graphics} start={start}");
        }

        // Order: after Lilu and the other built-in kexts, before the extra kexts; MinKernel 25.0.0.
        let mut c = Config::defaults();
        c.version = "tahoe".into();
        c.graphics = GRAPHICS_REIMS.into();
        let st = BootState {
            reims_kext: true,
            extra_kexts: vec!["Foo".into()],
            ..Default::default()
        };
        let cfg = opencore_config(
            &BootAssets::Embedded,
            &c,
            &Identity::generate()?,
            &BootOptions {
                keyboard: "en-US:0",
                ..Default::default()
            },
            &st,
        )?;
        let pos = |k: &str| cfg.find(&format!("<string>{k}.kext</string>")).unwrap_or(0);
        let (lilu, weg, i, foo) = (
            pos("Lilu"),
            pos("WhateverGreen"),
            pos("ReimsVgpu"),
            pos("Foo"),
        );
        assert!(
            lilu > 0
                && i > lilu
                && i > weg
                && foo > i
                && cfg[i..foo].contains("<string>25.0.0</string>")
        );
        Ok(())
    }
}

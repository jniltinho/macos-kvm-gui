//! The boot files a VM's workspace gets: OpenCore's EFI tree, OVMF, the NVRAM, the SMBIOS
//! identity and config.plist (Go `boot.go`).

use std::fs;
use std::path::{Path, PathBuf};

use rg_core::Config;
use serde::{Deserialize, Serialize};

use crate::assets::BootAssets;
use crate::compat::{ExtraKexts, reims_kext};
use crate::rootpatch::{ROOT_PATCH_BOOT_ARGS, ROOT_PATCH_MARKER, kext_entry, root_patch_plist};
use crate::util::{base64, copy_tree, exists, hex, hex_decode, random_bytes, write_atomic};
use crate::{Context, Result, extrakexts, invalid, reimskext, rootpatch};

/// The app's folder in a VM's workspace.
pub const BOOT_DIR: &str = "macos-kvm-gui";
/// OpenCore's tree, exposed to QEMU as a FAT disk (vvfat).
pub const EFI_DIR: &str = "macos-kvm-gui/efi";
/// The firmware code.
pub const OVMF_CODE: &str = "macos-kvm-gui/OVMF_CODE_4M.fd";
/// The VM's NVRAM: created once, never overwritten.
pub const VARS_FILE: &str = "macos-kvm-gui/OVMF_VARS_4M.fd";
const IDENTITY_FILE: &str = "macos-kvm-gui/smbios.json";
const RESOLUTION: &str = "1920x1080@32";
/// OC_SCAN_FILE_SYSTEM_LOCK | OC_SCAN_ALLOW_FS_APFS | OC_SCAN_ALLOW_FS_HFS: hides OpenCore's
/// own FAT disk, so the installed system is the default entry, and still finds HFS+ High
/// Sierra installs.
const SCAN_INSTALLED: u32 = 0x301;

/// The VM's SMBIOS identity, generated once per workspace (`smbios.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Identity {
    pub serial: String,
    pub mlb: String,
    pub uuid: String,
    /// 6 bytes, hex.
    pub rom: String,
}

const SERIAL_CHARS: &[u8] = b"0123456789ABCDEFGHJKLMNPQRSTUVWXYZ";

fn random_string(n: usize) -> Result<String> {
    Ok(random_bytes(n)?
        .into_iter()
        .map(|b| char::from(SERIAL_CHARS[usize::from(b) % SERIAL_CHARS.len()]))
        .collect())
}

impl Identity {
    /// A new random identity: a version 4 UUID, a 6-byte ROM, a 12-character serial and a
    /// 17-character MLB.
    pub fn generate() -> Result<Self> {
        let mut u = random_bytes(16 + 6)?;
        u[6] = (u[6] & 0x0f) | 0x40;
        u[8] = (u[8] & 0x3f) | 0x80;
        let h = hex(&u[..16]).to_uppercase();
        Ok(Self {
            uuid: format!(
                "{}-{}-{}-{}-{}",
                &h[..8],
                &h[8..12],
                &h[12..16],
                &h[16..20],
                &h[20..]
            ),
            rom: hex(&u[16..]),
            serial: random_string(12)?,
            mlb: random_string(17)?,
        })
    }

    /// The workspace's identity, created on first use.
    pub fn load_or_create(ws_dir: &Path) -> Result<Self> {
        let path = ws_dir.join(IDENTITY_FILE);
        match fs::read(&path) {
            Ok(data) => Ok(serde_json::from_slice(&data)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let id = Self::generate()?;
                write_atomic(&path, serde_json::to_string_pretty(&id)?.as_bytes(), 0o644)?;
                Ok(id)
            }
            Err(e) => Err(e.into()),
        }
    }
}

/// The kext caches shared by a Workspace's VMs: `<Workspace>/.kexts`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KextCache {
    root: PathBuf,
}

impl KextCache {
    /// `workspace_dir` is the Workspace folder, `~` already expanded (rg-core's
    /// `WorkspaceRoot()` through `WorkspaceDir()`).
    pub fn new(workspace_dir: &Path) -> Self {
        Self {
            root: workspace_dir.join(".kexts"),
        }
    }

    /// OCLP's root-patch kexts: `.kexts/oclp-2.5.1`.
    pub fn root_patch_dir(&self) -> PathBuf {
        self.root.join("oclp-2.5.1")
    }

    /// `.kexts/reims-util-b302389/ReimsVgpu.kext`.
    pub fn reims_kext_dir(&self) -> PathBuf {
        self.root
            .join(format!("reims-util-{}", &reimskext::REIMS_UTIL_COMMIT[..7]))
            .join("ReimsVgpu.kext")
    }

    /// settings.yaml's kexts: `.kexts/extra`.
    pub fn extra_dir(&self) -> PathBuf {
        self.root.join("extra")
    }
}

/// Go `SMBIOSModel`: a Mac that supports the macOS version. iMac models also keep
/// AppleMCEReporter (which panics on AMD hosts) from loading.
pub fn smbios_model(version: &str) -> &'static str {
    match version {
        "high-sierra" => "iMac18,3",
        "mojave" => "iMac19,1",
        _ => "iMac20,1", // Catalina 10.15.6 to Tahoe
    }
}

/// A boot's options that are not saved with the VM.
#[derive(Clone, Copy, Debug, Default)]
pub struct BootOptions<'a> {
    /// Install media attached: show the picker and scan every file system.
    pub with_media: bool,
    /// macOS prints its boot log on screen (`-v debug=0x100`).
    pub verbose_boot: bool,
    /// A start's settings.yaml snapshot; `None` (creating a VM) loads no extra kext and stays
    /// offline.
    pub extras: Option<&'a ExtraKexts>,
    /// NVRAM `prev-lang:kbd`, e.g. `en-US:128` (rg-core's `recovery_keyboard()`).
    pub keyboard: &'a str,
}

/// What [`prepare_boot`] found and passes to [`opencore_config`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BootState {
    /// The previous boot had root-patch mode on (its marker is there) and this one has it off.
    pub root_patch_was_on: bool,
    /// ReimsVgpu.kext was copied into the EFI.
    pub reims_kext: bool,
    /// settings.yaml kexts copied into the EFI, in load order.
    pub extra_kexts: Vec<String>,
}

/// Go `OpenCoreConfig`: renders config.plist. With install media the picker is shown and
/// every file system scanned so the installer can be chosen; otherwise OpenCore boots straight
/// into the only APFS entry.
pub fn opencore_config(
    assets: &BootAssets,
    c: &Config,
    id: &Identity,
    opts: &BootOptions<'_>,
    state: &BootState,
) -> Result<String> {
    let tmpl = assets.read("config.plist.tmpl")?;
    let tmpl = String::from_utf8_lossy(&tmpl);
    let rom = match hex_decode(&id.rom) {
        Some(r) if r.len() == 6 => r,
        _ => return invalid(format!("invalid SMBIOS ROM {:?}", id.rom)),
    };
    let disabled = opts
        .extras
        .map(|e| e.disabled.as_slice())
        .unwrap_or_default();
    let mut text = extrakexts::kext_switches(&tmpl, disabled)?;
    let (picker, timeout, scan) = if opts.with_media {
        ("<true/>", "5", "0".to_string())
    } else {
        ("<false/>", "0", SCAN_INSTALLED.to_string())
    };
    let mut boot_args = "keepsyms=1".to_string();
    if opts.verbose_boot {
        boot_args += " -v debug=0x100";
    }
    if !c.gpu.is_empty() {
        boot_args += " agdpmod=pikera"; // Navi cards show a black screen on iMac board IDs without it
    }
    if c.root_patch {
        boot_args += ROOT_PATCH_BOOT_ARGS;
    }
    let mut plist = root_patch_plist(c.root_patch, state.root_patch_was_on);
    if state.reims_kext {
        plist.kexts += &kext_entry(
            "ReimsVgpu",
            reimskext::REIMS_KEXT_MIN_KERNEL,
            &format!(
                "ReimsVgpu.kext (Reims vGPU on macOS 26, from reims-util {})",
                &reimskext::REIMS_UTIL_COMMIT[..7]
            ),
        );
    }
    for k in &state.extra_kexts {
        plist.kexts += &kext_entry(k, "", &format!("{k}.kext (settings.yaml)"));
    }
    // Go renders these in one pass (strings.Replacer); no value contains "@@", so replacing
    // them one by one gives the same text.
    for (k, v) in [
        ("@@CSR@@", plist.csr),
        ("@@ROOTPATCHKEXTS@@", &plist.kexts),
        ("@@CSRDELETE@@", plist.csr_delete),
        ("@@REVBLOCKADD@@", plist.revblock_add),
        ("@@REVBLOCKDELETE@@", plist.revblock_delete),
        ("@@SHOWPICKER@@", picker),
        ("@@TIMEOUT@@", timeout),
        ("@@SCANPOLICY@@", &scan),
        ("@@BOOTARGS@@", &boot_args),
        ("@@PREVLANG@@", &base64(opts.keyboard.as_bytes())),
        ("@@PRODUCT@@", smbios_model(&c.version)),
        ("@@SERIAL@@", &id.serial),
        ("@@MLB@@", &id.mlb),
        ("@@UUID@@", &id.uuid),
        ("@@ROM@@", &base64(&rom)),
        ("@@RESOLUTION@@", RESOLUTION),
    ] {
        text = text.replace(k, v);
    }
    Ok(text)
}

/// Go `Workspace.PrepareBoot`: refreshes the boot files and config.plist in the workspace
/// `ws_dir`. The NVRAM and the SMBIOS identity are created once and kept. With a start's
/// snapshot (`opts.extras`), the cached ReimsVgpu.kext (Tahoe on Reims vGPU) and the
/// settings.yaml kexts are copied in; root-patch mode needs the OCLP kexts cached.
pub fn prepare_boot(
    ws_dir: &Path,
    c: &Config,
    assets: &BootAssets,
    cache: &KextCache,
    opts: &BootOptions<'_>,
) -> Result<()> {
    let efi = ws_dir.join(EFI_DIR);
    fs::create_dir_all(ws_dir.join(BOOT_DIR))?;
    match fs::remove_dir_all(&efi) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    extract_efi(assets, &efi).context(|| "extracting OpenCore".into())?;
    fs::write(
        ws_dir.join(OVMF_CODE),
        assets.read("files/ovmf/OVMF_CODE_4M.fd")?,
    )?;
    let vars = ws_dir.join(VARS_FILE);
    if !exists(&vars) {
        // A half-written NVRAM would never be retried, so it is published only once complete.
        write_atomic(&vars, &assets.read("files/ovmf/OVMF_VARS_4M.fd")?, 0o644)?;
    }
    let id = Identity::load_or_create(ws_dir).context(|| "SMBIOS identity".into())?;
    let kexts_dir = efi.join("EFI/OC/Kexts");
    if c.root_patch {
        rootpatch::add_root_patch_kexts(cache, &kexts_dir)?;
    }
    let mut state = BootState::default();
    // Only at a start (extras set), where the kext was downloaded first: creating a VM stays
    // offline, and config.plist is written again at every start.
    if reims_kext(c) && opts.extras.is_some() {
        reimskext::add_reims_kext(cache, &kexts_dir)?;
        state.reims_kext = true;
    }
    if let Some(extras) = opts.extras.filter(|e| !e.entries.is_empty()) {
        let builtin: Vec<String> = assets
            .entries("files/EFI/OC/Kexts")?
            .into_iter()
            .filter_map(|(p, dir)| {
                let name = p
                    .strip_prefix("files/EFI/OC/Kexts/")?
                    .strip_suffix(".kext")?;
                (dir && !name.contains('/')).then(|| name.to_string())
            })
            .collect();
        state.extra_kexts = extrakexts::add_extra_kexts(extras, cache, &builtin, &kexts_dir)?;
    }
    state.root_patch_was_on = !c.root_patch && exists(&ws_dir.join(ROOT_PATCH_MARKER));
    let cfg = opencore_config(assets, c, &id, opts, &state)?;
    fs::write(efi.join("EFI/OC/config.plist"), cfg)?;
    // While the marker is there, boots with the mode off delete the lowered SIP; it is removed
    // only after such a boot really ran (clear_root_patch_marker), not when a config is merely
    // written (workspace create, a QEMU that fails to start).
    if c.root_patch {
        fs::write(ws_dir.join(ROOT_PATCH_MARKER), "on\n")?;
    }
    // OpenCore's own disk would show in its picker as "QEMU VVFAT", ahead of macOS Base
    // System. .contentVisibility "Disabled" hides that volume's entries only, so the recovery
    // or the installed system is the first, default entry. (A stricter ScanPolicy would hide
    // it too, but the file system lock stops the Sonoma recovery at the prohibited sign.)
    for p in [".contentVisibility", "EFI/BOOT/.contentVisibility"] {
        fs::write(efi.join(p), "Disabled")?;
    }
    Ok(())
}

fn extract_efi(assets: &BootAssets, efi: &Path) -> Result<()> {
    fs::create_dir_all(efi)?;
    for (p, dir) in assets.entries("files/EFI")? {
        let Some(rel) = p.strip_prefix("files/") else {
            continue;
        };
        let dst = efi.join(rel);
        if dir {
            fs::create_dir_all(&dst)?;
        } else {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&dst, assets.read(&p)?)?;
        }
    }
    Ok(())
}

/// Copies a cached kext folder into the EFI's Kexts folder.
pub(crate) fn copy_kext(src: &Path, kexts_dir: &Path, name: &str) -> Result<()> {
    copy_tree(src, &kexts_dir.join(format!("{name}.kext")))
        .context(|| format!("copying {name}.kext"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::testutil::tempdir;

    fn identity() -> Result<Identity> {
        Identity::generate()
    }

    fn render(c: &Config, opts: &BootOptions<'_>) -> Result<String> {
        opencore_config(
            &BootAssets::Embedded,
            c,
            &identity()?,
            opts,
            &BootState::default(),
        )
    }

    const KBD: &str = "en-US:0";

    #[test]
    fn identity_has_go_shapes() -> Result<()> {
        let id = identity()?;
        assert_eq!(
            (id.serial.len(), id.mlb.len(), id.uuid.len(), id.rom.len()),
            (12, 17, 36, 12)
        );
        assert_eq!(&id.uuid[14..15], "4");
        assert!(id.serial.bytes().all(|b| SERIAL_CHARS.contains(&b)));
        Ok(())
    }

    #[test]
    fn opencore_config_picker_scan_and_boot_args() -> Result<()> {
        let id = identity()?;
        let mut c = Config::defaults();
        let r = |c: &Config, o: &BootOptions<'_>| {
            opencore_config(&BootAssets::Embedded, c, &id, o, &BootState::default())
        };
        let media = BootOptions {
            with_media: true,
            keyboard: KBD,
            ..Default::default()
        };
        let plain = BootOptions {
            keyboard: KBD,
            ..Default::default()
        };
        let install = r(&c, &media)?;
        let installed = r(&c, &plain)?;
        c.gpu = "0000:03:00.0".into();
        let gpu = r(&c, &plain)?;
        for cfg in [&install, &installed, &gpu] {
            assert!(!cfg.contains("@@"), "unreplaced placeholder");
            assert!(
                cfg.contains("<string>iMac20,1</string>")
                    && cfg.contains(&format!("<string>{}</string>", id.serial))
            );
            assert!(cfg.contains("1920x1080@32"));
        }
        assert!(
            install.contains("<key>ShowPicker</key>\n\t\t\t<true/>")
                && install.contains("<integer>0</integer>")
        );
        assert!(
            installed.contains("<key>ShowPicker</key>\n\t\t\t<false/>")
                && installed.contains("<integer>769</integer>")
        );
        assert!(installed.contains("<key>MinDate</key>\n\t\t\t<integer>-1</integer>"));
        assert!(installed.contains("<key>MinVersion</key>\n\t\t\t<integer>-1</integer>"));
        c.gpu.clear();
        let verbose = r(
            &c,
            &BootOptions {
                verbose_boot: true,
                ..plain
            },
        )?;
        assert!(verbose.contains("keepsyms=1 -v debug=0x100") && !installed.contains("-v"));
        assert!(gpu.contains("keepsyms=1 agdpmod=pikera") && !installed.contains("agdpmod"));
        Ok(())
    }

    #[test]
    fn smbios_model_per_version() {
        assert_eq!(smbios_model("high-sierra"), "iMac18,3");
        assert_eq!(smbios_model("mojave"), "iMac19,1");
        assert_eq!(smbios_model("tahoe"), "iMac20,1");
    }

    #[test]
    fn recovery_keyboard_goes_to_prev_lang_and_nvram_delete() -> Result<()> {
        let cfg = render(
            &Config::defaults(),
            &BootOptions {
                with_media: true,
                keyboard: "en-US:128",
                ..Default::default()
            },
        )?;
        assert!(cfg.contains("<key>prev-lang:kbd</key>\n\t\t\t\t<data>\n\t\t\t\tZW4tVVM6MTI4\n"));
        // Add never overwrites an existing variable: without Delete, the first boot's value stays.
        let del = cfg
            .split("<key>Delete</key>\n\t\t<dict>\n\t\t\t<key>4D1EDE05")
            .nth(1)
            .unwrap_or("");
        assert!(
            del.split("</dict>")
                .next()
                .unwrap_or("")
                .contains("<string>prev-lang:kbd</string>")
        );
        Ok(())
    }

    #[test]
    fn prepare_extracts_boot_files_once() -> Result<()> {
        let ws = tempdir();
        let cache = KextCache::new(ws.path());
        let c = Config::defaults();
        let opts = BootOptions {
            keyboard: KBD,
            ..Default::default()
        };
        prepare_boot(ws.path(), &c, &BootAssets::Embedded, &cache, &opts)?;
        for f in [
            "EFI/BOOT/BOOTx64.efi",
            "EFI/OC/OpenCore.efi",
            "EFI/OC/config.plist",
            "EFI/OC/Kexts/Lilu.kext/Contents/MacOS/Lilu",
            "EFI/OC/Drivers/OpenRuntime.efi",
        ] {
            assert!(
                exists(&ws.path().join(EFI_DIR).join(f)),
                "{f} was not extracted"
            );
        }
        for f in [OVMF_CODE, VARS_FILE, IDENTITY_FILE] {
            assert!(exists(&ws.path().join(f)), "{f} was not extracted");
        }
        let id1 = fs::read(ws.path().join(IDENTITY_FILE))?;
        fs::write(ws.path().join(VARS_FILE), "modified-by-guest")?;
        prepare_boot(ws.path(), &c, &BootAssets::Embedded, &cache, &opts)?;
        assert_eq!(
            fs::read(ws.path().join(IDENTITY_FILE))?,
            id1,
            "the SMBIOS identity is generated once"
        );
        assert_eq!(
            fs::read(ws.path().join(VARS_FILE))?,
            b"modified-by-guest",
            "the NVRAM is never overwritten"
        );
        let id: Identity = serde_json::from_slice(&id1)?;
        assert_eq!(
            String::from_utf8_lossy(&id1),
            serde_json::to_string_pretty(&id)?
        );
        Ok(())
    }

    #[test]
    fn prepare_hides_opencore_volume() -> Result<()> {
        let ws = tempdir();
        let opts = BootOptions {
            with_media: true,
            keyboard: KBD,
            ..Default::default()
        };
        prepare_boot(
            ws.path(),
            &Config::defaults(),
            &BootAssets::Embedded,
            &KextCache::new(ws.path()),
            &opts,
        )?;
        for p in [".contentVisibility", "EFI/BOOT/.contentVisibility"] {
            assert_eq!(
                fs::read_to_string(ws.path().join(EFI_DIR).join(p))?,
                "Disabled"
            );
        }
        Ok(())
    }

    #[test]
    fn prepare_from_assets_dir_override() -> Result<()> {
        let ws = tempdir();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let assets = BootAssets::open(Some(&dir))?;
        let opts = BootOptions {
            keyboard: KBD,
            ..Default::default()
        };
        prepare_boot(
            ws.path(),
            &Config::defaults(),
            &assets,
            &KextCache::new(ws.path()),
            &opts,
        )?;
        assert!(exists(&ws.path().join(EFI_DIR).join("EFI/OC/OpenCore.efi")));
        Ok(())
    }
}

//! QEMU's command line for a VM: OpenCore-Boot.sh ported, for VMware SVGA and Reims vGPU (Go
//! `vm.go` `BuildArgs`, `reimshost.go` `reimsArgs`, `network.go` `netArgs`).

use rg_core::Config;
use rg_core::network::net_args;
use rg_core::workspace::{DISK_FILE, EFI_DIR, MEDIA_FILE, OVMF_CODE, VARS_FILE};
use std::path::{Path, PathBuf};

/// isa-applesmc's key, as in OpenCore-Boot.sh.
pub const OSK: &str = "ourhardworkbythesewordsguardedpleasedontsteal(c)AppleComputerInc";

/// What [`build_args`] needs besides the VM's config. Host lookups are fields, not calls, so
/// the arguments are reproducible (tests, golden files); [`ArgsEnv::host`] fills them in.
#[derive(Clone, Debug, Default)]
pub struct ArgsEnv {
    /// The VM's folder.
    pub workspace: PathBuf,
    /// The run's QMP socket; the VNC and control sockets sit next to it.
    pub qmp_socket: PathBuf,
    /// Attach the recovery image (`BaseSystem.img`).
    pub with_media: bool,
    /// The cached full installer to attach as a disc with the recovery (`CachedInstallerFor`).
    pub installer_disc: Option<PathBuf>,
    /// PCI functions of a passed-through GPU, the GPU first.
    pub gpu_funcs: Vec<String>,
    /// qemu-bridge-helper's path, or empty.
    pub bridge_helper: String,
    /// Reims vGPU's audio backend: `pipewire`, `pa` or `none`.
    pub audio: String,
    /// VMware SVGA only: run QEMU's own SDL window (`-display sdl`) for the app to embed, plus
    /// the VNC socket. Off (the default) gives the Go app's arguments.
    pub qemu_window: bool,
}

impl ArgsEnv {
    /// An environment with the host's bridge helper and sound server.
    pub fn host(workspace: impl Into<PathBuf>, qmp_socket: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            qmp_socket: qmp_socket.into(),
            bridge_helper: rg_core::network::bridge_helper().unwrap_or_default().into(),
            audio: reims_audio().into(),
            ..Self::default()
        }
    }
}

/// `reimsAudio`: the host's sound server, PipeWire, PulseAudio or none.
pub fn reims_audio() -> &'static str {
    let Some(run) = std::env::var_os("XDG_RUNTIME_DIR").filter(|r| !r.is_empty()) else {
        return "none";
    };
    let run = PathBuf::from(run);
    if run.join("pipewire-0").exists() {
        "pipewire"
    } else if run.join("pulse/native").exists() {
        "pa"
    } else {
        "none"
    }
}

/// Escapes commas, which separate QEMU options.
pub fn qemu_path(p: impl AsRef<Path>) -> String {
    p.as_ref().to_string_lossy().replace(',', ",,")
}

/// The second QMP socket next to a run's QMP socket.
pub fn control_socket(qmp_socket: &Path) -> PathBuf {
    sibling(qmp_socket, "qmp-control.sock")
}

/// The VNC socket next to a run's QMP socket.
pub fn vnc_socket(qmp_socket: &Path) -> PathBuf {
    sibling(qmp_socket, "vnc.sock")
}

fn sibling(p: &Path, name: &str) -> PathBuf {
    p.parent().unwrap_or(Path::new("")).join(name)
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| (*x).to_string()).collect()
}

/// The arguments of `qemu-system-x86_64` for `c` (Go `BuildArgs`, byte for byte).
pub fn build_args(c: &Config, env: &ArgsEnv) -> Vec<String> {
    let f = |name: &str| qemu_path(env.workspace.join(name));
    let mut args = s(&["-name", "macOS", "-enable-kvm", "-m"]);
    args.extend([
        c.ram_mib.to_string(),
        "-cpu".into(),
        rg_core::config::profile_for(&c.version).cpu,
    ]);
    args.extend(s(&[
        "-machine",
        "q35",
        "-device",
        "qemu-xhci,id=xhci",
        "-device",
        "usb-kbd,bus=xhci.0",
        "-device",
        "usb-tablet,bus=xhci.0",
        "-smp",
    ]));
    let threads = c.vcpus.checked_div(c.cores).unwrap_or(0);
    args.push(format!(
        "{},sockets=1,cores={},threads={threads}",
        c.vcpus, c.cores
    ));
    args.extend(s(&["-device", "usb-ehci,id=ehci", "-device"]));
    args.push(format!("isa-applesmc,osk={OSK}"));
    args.extend([
        "-drive".into(),
        format!("if=pflash,format=raw,readonly=on,file={}", f(OVMF_CODE)),
        "-drive".into(),
        format!("if=pflash,format=raw,file={}", f(VARS_FILE)),
    ]);
    args.extend(s(&[
        "-smbios",
        "type=2",
        "-device",
        "ich9-intel-hda",
        "-device",
        "hda-duplex",
        "-device",
        "ich9-ahci,id=sata",
    ]));
    // OpenCore boots from the extracted EFI directory, exposed read-only as a FAT disk by
    // QEMU's vvfat driver. SATA refuses read-only drives, so it sits on virtio-blk.
    args.extend([
        "-drive".into(),
        format!(
            "id=OpenCoreBoot,if=none,format=raw,readonly=on,file=fat:{}",
            f(EFI_DIR)
        ),
        "-device".into(),
        "virtio-blk-pci,drive=OpenCoreBoot,bootindex=0".into(),
    ]);
    if c.reims() {
        args = reims_args(args, c.ram_mib, &env.audio);
    }
    if env.with_media {
        args.extend([
            "-device".into(),
            "ide-hd,bus=sata.3,drive=InstallMedia".into(),
            "-drive".into(),
            format!("id=InstallMedia,if=none,format=raw,file={}", f(MEDIA_FILE)),
        ]);
        // The cached full installer as a disc: the recovery installs from it with no network.
        // SATA refuses read-only drives; snapshot=on keeps the guest's writes off the shared file.
        if let Some(disc) = &env.installer_disc {
            args.extend([
                "-device".into(),
                "ide-hd,bus=sata.2,drive=InstallerDisc".into(),
                "-drive".into(),
                format!(
                    "id=InstallerDisc,if=none,format=raw,snapshot=on,file={}",
                    qemu_path(disc)
                ),
            ]);
        }
    }
    args.extend([
        "-drive".into(),
        format!("id=MacHDD,if=none,format=qcow2,file={}", f(DISK_FILE)),
        "-device".into(),
        "ide-hd,bus=sata.4,drive=MacHDD".into(),
    ]);
    let (first_nic, more_nics) = net_args(c, &env.bridge_helper);
    args.extend(first_nic);
    // vmware-svga stays even with a passed-through GPU: QEMU's window needs a graphical
    // console to deliver keyboard and mouse input, and it shows the boot screens.
    args.push("-device".into());
    if c.reims() {
        // No addr: upstream's 0x4 is taken by ich9-ahci, and OpenCore boots by scanning.
        args.push(format!(
            "apple-gfx-pci,id=gfx,displays=1,resolution={}",
            c.reims_resolution()
        ));
    } else {
        args.push("vmware-svga".into());
    }
    if let Some((gpu, rest)) = env.gpu_funcs.split_first() {
        args.extend([
            "-device".into(),
            format!("vfio-pci,host={gpu},multifunction=on,x-no-kvm-intx=on"),
        ]);
        for f in rest {
            args.extend(["-device".into(), format!("vfio-pci,host={f}")]);
        }
    }
    args.extend(more_nics);
    if env.qemu_window && !c.reims() {
        // QEMU's own GTK window, for the app to embed (native scancodes); VNC still serves the
        // Preview and screen capture. GTK, not SDL: QEMU's SDL display withdraws its window
        // when it has no text consoles and paints "parallel0 console" over the guest when it
        // has them. No menu bar or tabs, and no serial/parallel consoles to switch to.
        args.extend([
            "-serial".into(),
            "none".into(),
            "-parallel".into(),
            "none".into(),
            "-display".into(),
            "gtk,show-tabs=off,show-menubar=off,zoom-to-fit=on".into(),
            "-vnc".into(),
            format!(
                "unix:{},lock-key-sync=off",
                qemu_path(vnc_socket(&env.qmp_socket))
            ),
        ]);
    } else if c.in_app() || c.reims() {
        // A Reims VM's screen is the library's own window (embedded by the app); the VNC socket
        // still serves the Preview and screen capture. No QEMU window: the screen goes to the
        // app over VNC on a Unix socket in the same private 0700 directory as QMP.
        // No -k: with it, QEMU remaps extended key events through that layout's keysyms and
        // drops the physical scancode. lock-key-sync=off keeps QEMU from injecting Caps Lock
        // presses inferred from keysyms.
        args.extend([
            "-display".into(),
            "none".into(),
            "-vnc".into(),
            format!(
                "unix:{},lock-key-sync=off",
                qemu_path(vnc_socket(&env.qmp_socket))
            ),
        ]);
    }
    // A second monitor for one other process (the MCP server or an attached app): QMP serves
    // one client per monitor, and the starter keeps the first.
    args.extend([
        "-qmp".into(),
        format!("unix:{},server=on,wait=off", qemu_path(&env.qmp_socket)),
        "-qmp".into(),
        format!(
            "unix:{},server=on,wait=off",
            qemu_path(control_socket(&env.qmp_socket))
        ),
    ]);
    args
}

/// Rewrites the common arguments the way upstream's measured x86-64 command differs: guest RAM
/// as a shared memfd (the library maps it), no ACPI S3, no default VGA, xHCI interrupt mapping
/// for macOS's driver, and one audio backend bound to the HDA codec and a USB audio device
/// (macOS 26 has no HDA driver).
fn reims_args(args: Vec<String>, ram_mib: u32, audio: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len() + 16);
    for a in args {
        match a.as_str() {
            "q35" => {
                out.push("q35,memory-backend=ram".into());
                out.push("-object".into());
                out.push(format!(
                    "memory-backend-memfd,id=ram,size={ram_mib}M,share=on"
                ));
                out.extend(s(&["-global", "ICH9-LPC.disable_s3=1", "-vga", "none"]));
            }
            "qemu-xhci,id=xhci" => out.push(format!("{a},conditional-intr-mapping=on")),
            "hda-duplex" => {
                // An explicit audiodev leaves no default driver for an unbound codec: bind both.
                out.push("hda-duplex,audiodev=snd0".into());
                out.push("-audiodev".into());
                out.push(format!("{audio},id=snd0"));
                out.extend(s(&["-device", "usb-audio,audiodev=snd0,bus=xhci.0"]));
            }
            _ => out.push(a),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rg_core::Nic;

    fn arg_after<'a>(args: &'a [String], flag: &str, prefix: &str) -> &'a str {
        args.windows(2)
            .find(|w| w[0] == flag && w[1].starts_with(prefix))
            .map_or("", |w| w[1].as_str())
    }

    fn env(with_media: bool) -> ArgsEnv {
        ArgsEnv {
            workspace: "/vm/a,b".into(),
            qmp_socket: "/run/q.sock".into(),
            with_media,
            audio: "pipewire".into(),
            ..ArgsEnv::default()
        }
    }

    #[test]
    fn build_args_per_version() {
        let mut c = Config::defaults();
        for (name, version, media, cpu, nic) in [
            (
                "sonoma with media",
                "sonoma",
                true,
                "Skylake-Client",
                "virtio-net-pci",
            ),
            (
                "sonoma without media",
                "sonoma",
                false,
                "Skylake-Client",
                "virtio-net-pci",
            ),
            ("high sierra", "high-sierra", false, "Penryn", "vmxnet3"),
            (
                "catalina has no VirtIO network driver",
                "catalina",
                false,
                "Penryn",
                "vmxnet3",
            ),
        ] {
            c.version = version.into();
            let args = build_args(&c, &env(media));
            assert!(
                arg_after(&args, "-cpu", "").starts_with(&format!("{cpu},")),
                "{name}"
            );
            assert!(
                !arg_after(&args, "-device", &format!("{nic},netdev=net0")).is_empty(),
                "{name}"
            );
            assert_eq!(
                !arg_after(&args, "-drive", "id=InstallMedia").is_empty(),
                media,
                "{name}"
            );
            assert_eq!(
                arg_after(&args, "-netdev", "user"),
                "user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22",
                "{name}"
            );
            assert!(
                arg_after(&args, "-vnc", "unix:").ends_with("vnc.sock,lock-key-sync=off"),
                "{name}"
            );
            assert_eq!(arg_after(&args, "-display", ""), "none", "{name}");
            assert_eq!(
                arg_after(&args, "-smp", ""),
                "8,sockets=1,cores=4,threads=2",
                "{name}"
            );
            assert!(
                arg_after(&args, "-drive", "id=MacHDD").ends_with("file=/vm/a,,b/mac_hdd_ng.img"),
                "{name}"
            );
            assert!(
                args.contains(&format!("if=pflash,format=raw,file=/vm/a,,b/{VARS_FILE}")),
                "{name}"
            );
            assert!(
                args.contains(&format!(
                    "id=OpenCoreBoot,if=none,format=raw,readonly=on,file=fat:/vm/a,,b/{EFI_DIR}"
                )),
                "{name}"
            );
        }
    }

    #[test]
    fn reims_args_rewrite_the_common_ones() {
        let mut c = Config::defaults();
        c.graphics = rg_core::config::GRAPHICS_REIMS.into();
        c.display = rg_core::config::DISPLAY_WINDOW.into();
        c.ram_mib = 16384;
        let mut e = env(false);
        e.workspace = "/vm/a".into();
        e.qmp_socket = "/run/macos-kvm-gui-x/qmp.sock".into();
        let joined = build_args(&c, &e).join(" ");
        for want in [
            "-m 16384 ",
            "-machine q35,memory-backend=ram -object memory-backend-memfd,id=ram,size=16384M,share=on",
            "-global ICH9-LPC.disable_s3=1",
            "-vga none",
            "-device apple-gfx-pci,id=gfx,displays=1,resolution=1920x1080 ",
            "-device qemu-xhci,id=xhci,conditional-intr-mapping=on ",
            "-device hda-duplex,audiodev=snd0 -audiodev pipewire,id=snd0 -device usb-audio,audiodev=snd0,bus=xhci.0",
            "-display none -vnc unix:/run/macos-kvm-gui-x/vnc.sock,lock-key-sync=off",
        ] {
            assert!(joined.contains(want), "missing {want:?} in {joined}");
        }
        assert!(
            !joined.contains("vmware-svga")
                && !joined.contains("addr=")
                && joined.matches("-audiodev").count() == 1
        );
    }

    #[test]
    fn installer_disc_rides_along_with_the_recovery() {
        let mut e = env(true);
        e.installer_disc = Some("/c/Install,X.iso".into());
        let args = build_args(&Config::defaults(), &e);
        assert_eq!(
            arg_after(&args, "-drive", "id=InstallerDisc"),
            "id=InstallerDisc,if=none,format=raw,snapshot=on,file=/c/Install,,X.iso"
        );
        e.with_media = false;
        assert!(
            arg_after(
                &build_args(&Config::defaults(), &e),
                "-drive",
                "id=InstallerDisc"
            )
            .is_empty()
        );
    }

    fn nic(enabled: bool, mode: &str, bridge: &str, model: &str, mac: &str) -> Nic {
        Nic {
            enabled,
            mode: mode.into(),
            bridge: bridge.into(),
            model: model.into(),
            mac: mac.into(),
        }
    }

    #[test]
    fn net_args_place_and_forward() {
        let mut c = Config::defaults();
        c.networks = Some(vec![
            nic(true, "nat", "", "virtio-net-pci", "52:54:00:00:00:01"),
            nic(true, "bridged", "br0", "vmxnet3", "52:54:00:00:00:02"),
            nic(false, "nat", "", "vmxnet3", "52:54:00:00:00:03"),
            nic(true, "nat", "", "vmxnet3", "52:54:00:00:00:04"),
        ]);
        let (first, rest) = net_args(&c, "/usr/lib/qemu/qemu-bridge-helper");
        let all = [first, rest].concat().join(" ");
        for want in [
            "user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22",
            "bridge,id=net1,br=br0,helper=/usr/lib/qemu/qemu-bridge-helper",
            "vmxnet3,netdev=net1,id=net1,mac=52:54:00:00:00:02,bus=pcie.0,addr=0x16",
            "user,id=net3,net=10.0.5.0/24",
            "vmxnet3,netdev=net3,id=net3,mac=52:54:00:00:00:04,bus=pcie.0,addr=0x18",
        ] {
            assert!(all.contains(want), "missing {want:?} in {all}");
        }
        assert!(
            !all.contains("net2") && all.matches("hostfwd").count() == 1,
            "{all}"
        );
        c.networks = Some(vec![nic(false, "nat", "", "vmxnet3", "52:54:00:00:00:01")]);
        assert_eq!(net_args(&c, "").0.join(" "), "-nic none");
    }

    #[test]
    fn qemu_window_keeps_vnc_for_svga_only() {
        let mut e = env(false);
        e.qemu_window = true;
        let args = build_args(&Config::defaults(), &e);
        assert_eq!(
            arg_after(&args, "-display", ""),
            "gtk,show-tabs=off,show-menubar=off,zoom-to-fit=on"
        );
        assert_eq!(arg_after(&args, "-serial", ""), "none");
        assert_eq!(arg_after(&args, "-parallel", ""), "none");
        assert!(arg_after(&args, "-vnc", "unix:").ends_with("vnc.sock,lock-key-sync=off"));
        let mut c = Config::defaults();
        c.graphics = rg_core::config::GRAPHICS_REIMS.into();
        assert_eq!(arg_after(&build_args(&c, &e), "-display", ""), "none");
    }

    #[test]
    fn sockets_sit_next_to_qmp() {
        let q = Path::new("/run/macos-kvm-gui-1/qmp.sock");
        assert_eq!(vnc_socket(q), Path::new("/run/macos-kvm-gui-1/vnc.sock"));
        assert_eq!(
            control_socket(q),
            Path::new("/run/macos-kvm-gui-1/qmp-control.sock")
        );
    }
}

//! Network adapters, like VirtualBox's Adapter 1–4: NAT (QEMU user mode) or bridged through
//! qemu-bridge-helper, each with a model and a MAC address (port of `network.go`).

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{Config, IDX_BIG_SUR, IDX_VENTURA, field_err, version_index};
use crate::{Error, Result, msg};

/// At most this many adapters.
pub const MAX_NICS: usize = 4;
pub const NET_NAT: &str = "nat";
pub const NET_BRIDGED: &str = "bridged";
/// The MAC every VM used before adapters were configurable (from OSX-KVM); legacy VMs keep it.
pub const LEGACY_MAC: &str = "52:54:00:c9:18:27";
/// The adapter types offered; see [`nic_supported`].
pub const NIC_MODELS: [&str; 3] = ["virtio-net-pci", "vmxnet3", "e1000-82545em"];
/// Where distributions install qemu-bridge-helper (Debian/Ubuntu, Fedora).
const BRIDGE_HELPERS: [&str; 2] = [
    "/usr/lib/qemu/qemu-bridge-helper",
    "/usr/libexec/qemu-bridge-helper",
];

/// One network adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nic {
    pub enabled: bool,
    /// [`NET_NAT`] or [`NET_BRIDGED`].
    pub mode: String,
    /// Host bridge for bridged mode, e.g. `br0`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bridge: String,
    pub model: String,
    pub mac: String,
}

/// Whether stock macOS of that version drives the model: VirtIO from Big Sur, the Intel 82545EM
/// until Monterey, vmxnet3 throughout.
pub fn nic_supported(version: &str, model: &str) -> bool {
    let i = version_index(version);
    match model {
        "virtio-net-pci" => i >= IDX_BIG_SUR,
        "e1000-82545em" => i < IDX_VENTURA,
        _ => model == "vmxnet3",
    }
}

/// The model a new adapter gets for the version.
pub fn default_nic_model(version: &str) -> &'static str {
    if version_index(version) >= IDX_BIG_SUR {
        "virtio-net-pci"
    } else {
        "vmxnet3"
    }
}

/// A new VM's adapters: one NAT adapter with a fresh MAC.
pub fn new_networks(version: &str) -> Vec<Nic> {
    vec![Nic {
        enabled: true,
        mode: NET_NAT.into(),
        model: default_nic_model(version).into(),
        mac: random_mac(),
        ..Nic::default()
    }]
}

/// A random locally administered unicast MAC address.
pub fn random_mac() -> String {
    let mut b = [0u8; 6];
    fastrand::fill(&mut b);
    b[0] = (b[0] & 0xfc) | 0x02;
    format_mac(&b)
}

fn format_mac(b: &[u8; 6]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Parses `aa:bb:cc:dd:ee:ff` (any case).
fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let mut out = [0u8; 6];
    let mut parts = s.split(':');
    for o in &mut out {
        let p = parts.next()?;
        if p.len() != 2 {
            return None;
        }
        *o = u8::from_str_radix(p, 16).ok()?;
    }
    parts.next().is_none().then_some(out)
}

impl Config {
    /// The effective adapters: the VM's own, or for a VM saved before adapters existed, one NAT
    /// adapter with the version's model and the legacy MAC.
    pub fn adapters(&self) -> Vec<Nic> {
        self.networks.clone().unwrap_or_else(|| {
            vec![Nic {
                enabled: true,
                mode: NET_NAT.into(),
                model: default_nic_model(&self.version).into(),
                mac: LEGACY_MAC.into(),
                ..Nic::default()
            }]
        })
    }

    /// Adapter `i` (0-based) for editing. A legacy VM gets its implicit adapter written out, and
    /// missing adapters up to `i` are added disabled with the version's model and a fresh MAC.
    pub fn nic_mut(&mut self, i: usize) -> Result<&mut Nic> {
        if i >= MAX_NICS {
            return Err(msg(format!("adapters are numbered 1 to {MAX_NICS}")));
        }
        let mut nics = self.adapters();
        while nics.len() <= i {
            nics.push(Nic {
                mode: NET_NAT.into(),
                model: default_nic_model(&self.version).into(),
                mac: random_mac(),
                ..Nic::default()
            });
        }
        Ok(&mut self.networks.insert(nics)[i])
    }

    /// Whether the VM has the localhost SSH forward (an enabled NAT adapter).
    pub fn ssh_forwarded(&self) -> bool {
        self.adapters()
            .iter()
            .any(|n| n.enabled && n.mode == NET_NAT)
    }
}

/// Go `bridgeNameRe` (`^[A-Za-z0-9_.-]{1,15}$`), also used for installer build names.
pub fn valid_bridge_name(s: &str) -> bool {
    (1..=15).contains(&s.len())
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
}

/// Checks the adapter list: known modes and models, a valid bridge name and unique, locally
/// administered unicast MACs. Field names are `networks` and `nic1`..`nic4`.
pub fn validate_nics(nics: &[Nic]) -> Result<()> {
    if nics.len() > MAX_NICS {
        return Err(field_err(
            "networks",
            format!("at most {MAX_NICS} adapters"),
        ));
    }
    let mut seen: HashMap<[u8; 6], usize> = HashMap::new();
    for (i, n) in nics.iter().enumerate() {
        let field = format!("nic{}", i + 1);
        if n.mode != NET_NAT && n.mode != NET_BRIDGED {
            return Err(field_err(field, "attached to must be nat or bridged"));
        }
        if n.mode == NET_BRIDGED && !valid_bridge_name(&n.bridge) {
            return Err(field_err(field, "choose the host bridge, for example br0"));
        }
        if !NIC_MODELS.contains(&n.model.as_str()) {
            return Err(field_err(
                field,
                format!("adapter type must be one of {}", NIC_MODELS.join(", ")),
            ));
        }
        let Some(hw) = parse_mac(&n.mac) else {
            return Err(field_err(
                field,
                format!("{} is not a MAC address such as 52:54:00:12:34:56", n.mac),
            ));
        };
        if hw[0] & 0x03 != 0x02 {
            return Err(field_err(
                field,
                format!(
                    "{} must be a locally administered unicast address (second hex digit 2, 6, A or E)",
                    n.mac
                ),
            ));
        }
        if let Some(j) = seen.insert(hw, i) {
            return Err(field_err(
                field,
                format!("{} is already used by adapter {}", n.mac, j + 1),
            ));
        }
    }
    Ok(())
}

/// QEMU escapes commas in option values by doubling them.
fn qemu_path(p: &str) -> String {
    p.replace(',', ",,")
}

/// The QEMU arguments for the adapters: `first` goes where adapter 1 always was, `rest`
/// (adapters 2–4 on fixed PCI slots) after the other devices. No enabled adapter gives
/// `-nic none`. The SSH forward goes on the first enabled NAT adapter; further NAT adapters get
/// their own subnet.
pub fn net_args(c: &Config, helper: &str) -> (Vec<String>, Vec<String>) {
    let (mut first, mut rest) = (Vec::new(), Vec::new());
    let (mut any, mut forwarded) = (false, false);
    for (i, n) in c.adapters().iter().enumerate() {
        if !n.enabled {
            continue;
        }
        any = true;
        let id = format!("net{i}");
        let netdev = if n.mode == NET_BRIDGED {
            format!(
                "bridge,id={id},br={},helper={}",
                n.bridge,
                qemu_path(helper)
            )
        } else {
            let mut s = format!("user,id={id}");
            if i > 0 {
                s += &format!(",net=10.0.{}.0/24", 2 + i);
            }
            if !forwarded {
                s += &format!(",hostfwd=tcp:127.0.0.1:{}-:22", c.ssh_port);
                forwarded = true;
            }
            s
        };
        let dev = format!(
            "{},netdev={id},id={id},mac={}",
            n.model,
            n.mac.to_lowercase()
        );
        if i == 0 {
            first.extend(["-netdev".into(), netdev, "-device".into(), dev]);
        } else {
            rest.extend([
                "-netdev".into(),
                netdev,
                "-device".into(),
                format!("{dev},bus=pcie.0,addr=0x{:x}", 0x15 + i),
            ]);
        }
    }
    if !any {
        first = vec!["-nic".into(), "none".into()];
    }
    (first, rest)
}

/// The installed qemu-bridge-helper, or `None`.
pub fn bridge_helper() -> Option<&'static str> {
    BRIDGE_HELPERS
        .into_iter()
        .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file()))
}

/// The host's Linux bridges.
pub fn host_bridges() -> Vec<String> {
    bridges_in(Path::new("/sys/class/net"))
}

/// The interfaces under `root` (a sysfs `class/net`) with a `bridge/` directory.
pub fn bridges_in(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().join("bridge").is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// The preflight for bridged adapters: refuses what certainly cannot work (no such bridge, no
/// helper, a bridge the ACL denies) and leaves the rest to QEMU. Normally called with
/// `/sys/class/net`, `/etc/qemu/bridge.conf` and [`bridge_helper`].
pub fn check_bridges(
    c: &Config,
    net_root: &Path,
    acl_file: &Path,
    helper: Option<&str>,
) -> Result<()> {
    for (i, n) in c.adapters().iter().enumerate() {
        if !n.enabled || n.mode != NET_BRIDGED {
            continue;
        }
        let name = format!("Adapter {}", i + 1);
        if !net_root.join(&n.bridge).join("bridge").is_dir() {
            return Err(msg(format!(
                "{name}: the host has no bridge named {} (see Troubleshooting › Bridged networking)",
                n.bridge
            )));
        }
        if helper.is_none_or(str::is_empty) {
            return Err(msg(format!(
                "{name}: qemu-bridge-helper is not installed (it comes with QEMU's common package)"
            )));
        }
        if bridge_allowed(acl_file, &n.bridge) == Some(false) {
            let acl = acl_file.display();
            return Err(msg(format!(
                "{name}: {acl} does not allow {b}. As root, add the line \"allow {b}\" to {acl}",
                b = n.bridge
            )));
        }
    }
    Ok(())
}

/// qemu-bridge-helper's ACL rules for `bridge` (`allow`, `deny`, `include`; a matching deny
/// wins). `None` when a file cannot be read, leaving the decision to the helper.
pub fn bridge_allowed(acl_file: &Path, bridge: &str) -> Option<bool> {
    fn read(
        path: &Path,
        bridge: &str,
        seen: &mut HashSet<PathBuf>,
        allow: &mut bool,
        deny: &mut bool,
    ) -> bool {
        if seen.contains(path) || seen.len() > 16 {
            return true;
        }
        seen.insert(path.to_path_buf());
        let mut text = String::new();
        if std::fs::File::open(path)
            .and_then(|mut f| f.read_to_string(&mut text))
            .is_err()
        {
            return false;
        }
        for line in text.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 2 || fields[0].starts_with('#') {
                continue;
            }
            let matched = fields[1] == "all" || fields[1] == bridge;
            match fields[0] {
                "allow" => *allow |= matched,
                "deny" => *deny |= matched,
                "include" if !read(Path::new(fields[1]), bridge, seen, allow, deny) => {
                    return false;
                }
                _ => {}
            }
        }
        true
    }
    let (mut allow, mut deny) = (false, false);
    read(acl_file, bridge, &mut HashSet::new(), &mut allow, &mut deny).then_some(allow && !deny)
}

/// Fails with [`Error::PortInUse`] when the SSH forward's port is taken on 127.0.0.1.
pub fn check_ssh_port(c: &Config) -> Result<()> {
    if c.ssh_forwarded() && std::net::TcpListener::bind(("127.0.0.1", c.ssh_port)).is_err() {
        return Err(Error::PortInUse(c.ssh_port));
    }
    Ok(())
}

/// The virtual size of a qcow2 disk from its header (bytes 24–31), or 0.
pub fn disk_virtual_size(path: &Path) -> u64 {
    let mut h = [0u8; 32];
    match std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut h)) {
        Ok(()) if &h[..4] == b"QFI\xfb" => {
            h[24..32].iter().fold(0, |s, &b| (s << 8) | u64::from(b))
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nic(mode: &str, bridge: &str, model: &str, mac: &str, enabled: bool) -> Nic {
        Nic {
            enabled,
            mode: mode.into(),
            bridge: bridge.into(),
            model: model.into(),
            mac: mac.into(),
        }
    }

    #[test]
    fn legacy_vm_keeps_its_network() {
        let (first, rest) = net_args(&Config::defaults(), "/usr/lib/qemu/qemu-bridge-helper");
        assert_eq!(
            first,
            [
                "-netdev",
                "user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22",
                "-device",
                "virtio-net-pci,netdev=net0,id=net0,mac=52:54:00:c9:18:27"
            ]
        );
        assert!(rest.is_empty());
    }

    #[test]
    fn net_args_for_four_adapters() {
        let mut c = Config::defaults();
        c.networks = Some(vec![
            nic("nat", "", "virtio-net-pci", "52:54:00:00:00:01", true),
            nic("bridged", "br0", "vmxnet3", "52:54:00:00:00:02", true),
            nic("nat", "", "vmxnet3", "52:54:00:00:00:03", false),
            nic("nat", "", "vmxnet3", "52:54:00:00:00:04", true),
        ]);
        let (first, rest) = net_args(&c, "/usr/lib/qemu/qemu-bridge-helper");
        let all = [first, rest].concat().join(" ");
        for s in [
            "user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22",
            "bridge,id=net1,br=br0,helper=/usr/lib/qemu/qemu-bridge-helper",
            "vmxnet3,netdev=net1,id=net1,mac=52:54:00:00:00:02,bus=pcie.0,addr=0x16",
            "user,id=net3,net=10.0.5.0/24",
            "vmxnet3,netdev=net3,id=net3,mac=52:54:00:00:00:04,bus=pcie.0,addr=0x18",
        ] {
            assert!(all.contains(s), "missing {s} in {all}");
        }
        assert!(!all.contains("net2") && all.matches("hostfwd").count() == 1);

        c.networks = Some(vec![nic("nat", "", "vmxnet3", "52:54:00:00:00:01", false)]);
        assert_eq!(net_args(&c, "").0, ["-nic", "none"]);
        assert!(!c.ssh_forwarded());
    }

    #[test]
    fn validate_nics_refuses_bad_adapters() {
        let ok = nic("nat", "", "vmxnet3", "52:54:00:12:34:56", true);
        let cases = [
            (
                "multicast",
                vec![nic("nat", "", "vmxnet3", "01:00:5e:00:00:01", true)],
            ),
            (
                "global",
                vec![nic("nat", "", "vmxnet3", "00:1c:42:00:00:01", true)],
            ),
            (
                "format",
                vec![nic("nat", "", "vmxnet3", "5254.0012.3456", true)],
            ),
            (
                "duplicate",
                vec![
                    ok.clone(),
                    nic("nat", "", "vmxnet3", "52:54:00:12:34:56", true),
                ],
            ),
            (
                "model",
                vec![nic("nat", "", "rtl8139", "52:54:00:12:34:56", true)],
            ),
            (
                "bridge name",
                vec![nic(
                    "bridged",
                    "br0;rm",
                    "vmxnet3",
                    "52:54:00:12:34:56",
                    true,
                )],
            ),
            ("too many", vec![ok.clone(); 5]),
        ];
        for (name, nics) in cases {
            assert!(validate_nics(&nics).is_err(), "{name} must be refused");
        }
        assert!(validate_nics(&[ok]).is_ok());
        for _ in 0..50 {
            let m = random_mac();
            assert!(parse_mac(&m).is_some_and(|hw| hw[0] & 3 == 2), "{m}");
        }
    }

    #[test]
    fn nic_driver_matrix() {
        for (version, model, ok) in [
            ("catalina", "virtio-net-pci", false),
            ("big-sur", "virtio-net-pci", true),
            ("monterey", "e1000-82545em", true),
            ("ventura", "e1000-82545em", false),
            ("high-sierra", "vmxnet3", true),
            ("tahoe", "vmxnet3", true),
        ] {
            assert_eq!(nic_supported(version, model), ok, "{model} on {version}");
        }
        assert_eq!(default_nic_model("mojave"), "vmxnet3");
        assert_eq!(default_nic_model("sonoma"), "virtio-net-pci");
    }

    #[test]
    fn nic_mut_writes_out_the_legacy_adapter() -> Result<()> {
        let mut c = Config::defaults();
        c.nic_mut(2)?.enabled = true;
        let nets = c.networks.clone().unwrap_or_default();
        assert_eq!(nets.len(), 3);
        assert_eq!(nets[0].mac, LEGACY_MAC);
        assert!(!nets[1].enabled && nets[2].enabled);
        assert!(c.nic_mut(4).is_err());
        Ok(())
    }

    #[test]
    fn bridge_acl_and_preflight() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let acl = dir.path().join("bridge.conf");
        let extra = dir.path().join("extra.conf");
        std::fs::write(&extra, "allow br1\n")?;
        std::fs::write(
            &acl,
            format!(
                "# comment\nallow br0\ninclude {}\ndeny br2\nallow all\n",
                extra.display()
            ),
        )?;
        for (br, want) in [
            ("br0", true),
            ("br1", true),
            ("br2", false),
            ("virbr0", true),
        ] {
            assert_eq!(bridge_allowed(&acl, br), Some(want), "{br}");
        }
        assert_eq!(bridge_allowed(&dir.path().join("missing"), "br0"), None);

        let net = tempfile::tempdir()?;
        std::fs::create_dir_all(net.path().join("br0/bridge"))?;
        std::fs::create_dir_all(net.path().join("eth0"))?;
        let mut c = Config::defaults();
        c.networks = Some(vec![nic(
            "bridged",
            "br2",
            "vmxnet3",
            "52:54:00:00:00:01",
            true,
        )]);
        std::fs::write(&acl, "allow br0\n")?;
        let err = |c: &Config, h| {
            check_bridges(c, net.path(), &acl, h)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default()
        };
        assert!(err(&c, Some("/helper")).contains("no bridge named br2"));
        if let Some(n) = c.networks.as_mut() {
            n[0].bridge = "br0".into();
        }
        assert!(err(&c, None).contains("qemu-bridge-helper"));
        std::fs::write(&acl, "deny br0\nallow all\n")?;
        assert!(err(&c, Some("/helper")).contains("allow br0"));
        std::fs::write(&acl, "allow br0\n")?;
        check_bridges(&c, net.path(), &acl, Some("/helper"))?;
        assert_eq!(bridges_in(net.path()), ["br0"]);
        Ok(())
    }

    #[test]
    fn ssh_port_in_use() -> Result<()> {
        let l = std::net::TcpListener::bind("127.0.0.1:0")?;
        let c = Config {
            ssh_port: l.local_addr()?.port(),
            ..Config::defaults()
        };
        assert!(matches!(check_ssh_port(&c), Err(Error::PortInUse(_))));
        Ok(())
    }

    #[test]
    fn qcow2_virtual_size() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let p = dir.path().join("d.img");
        let mut h = vec![0u8; 512];
        h[..4].copy_from_slice(b"QFI\xfb");
        h[24..32].copy_from_slice(&(64u64 << 20).to_be_bytes());
        std::fs::write(&p, &h)?;
        assert_eq!(disk_virtual_size(&p), 64 << 20);
        assert_eq!(disk_virtual_size(&dir.path().join("none")), 0);
        Ok(())
    }
}

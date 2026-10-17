//! Host checks and the one-prompt host setup (port of the Go app's `internal/core/host.go`).

use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rg_core::Config;

use rg_core::AppDir;

/// A check's outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Fail,
    /// The check could not tell.
    Unknown,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Status::Ok => "OK",
            Status::Warn => "Warning",
            Status::Fail => "Failure",
            Status::Unknown => "Inconclusive",
        })
    }
}

/// One line of the Host tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub detail: String,
    /// What the user can do about it ("" when nothing).
    pub hint: String,
}

impl Check {
    pub(crate) fn new(
        name: &str,
        status: Status,
        detail: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            status,
            detail: detail.into(),
            hint: hint.into(),
        }
    }
}

/// A command that failed: its exit code (`None` when it did not start or was killed) and its
/// combined output on one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CmdError {
    pub code: Option<i32>,
    pub output: String,
}

impl fmt::Display for CmdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code {
            Some(c) => write!(f, "exit status {c}")?,
            None => f.write_str("command failed")?,
        }
        if !self.output.is_empty() {
            write!(f, ": {}", self.output)?;
        }
        Ok(())
    }
}

impl std::error::Error for CmdError {}

/// This crate's errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HostError {
    /// The user closed or refused the password prompt.
    #[error("cancelled at the password prompt")]
    Cancelled,
    #[error("{0}")]
    Failed(String),
}

pub(crate) fn failed(msg: impl Into<String>) -> HostError {
    HostError::Failed(msg.into())
}

/// Finds a program in `PATH`.
pub type LookPathFn = Arc<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;
/// Runs a program and returns its combined output.
pub type OutputFn = Arc<dyn Fn(&str, &[&str]) -> Result<Vec<u8>, CmdError> + Send + Sync>;
/// Runs a program for its effect.
pub type RunFn = Arc<dyn Fn(&str, &[&str]) -> Result<(), CmdError> + Send + Sync>;
/// Finds the Reims vGPU QEMU.
pub type ReimsQemuFn = Arc<dyn Fn() -> Result<PathBuf, String> + Send + Sync>;
/// Reads settings.yaml's extra kexts: how many it lists.
pub type ExtraKextsFn = Arc<dyn Fn() -> Result<usize, String> + Send + Sync>;

/// The system, with every root and command swappable so tests run on fake trees.
#[derive(Clone)]
pub struct Host {
    pub proc_root: PathBuf,
    pub sys_root: PathBuf,
    pub etc_root: PathBuf,
    pub dev_kvm: PathBuf,
    pub dev_vfio: PathBuf,
    /// /usr/share (Vulkan ICD manifests).
    pub share_root: PathBuf,
    pub dev_udmabuf: PathBuf,
    pub look_path: LookPathFn,
    pub output: OutputFn,
    pub run: RunFn,
    pub reims_qemu: ReimsQemuFn,
    pub extra_kexts: ExtraKextsFn,
    /// settings.yaml's path, named in the patch-kext reason.
    pub settings_file: String,
}

impl Host {
    /// The real host.
    pub fn system() -> Self {
        Self {
            proc_root: "/proc".into(),
            sys_root: "/sys".into(),
            etc_root: "/etc".into(),
            dev_kvm: "/dev/kvm".into(),
            dev_vfio: "/dev/vfio".into(),
            share_root: "/usr/share".into(),
            dev_udmabuf: "/dev/udmabuf".into(),
            look_path: Arc::new(look_path),
            output: Arc::new(|n, a| {
                let o = Command::new(n).args(a).output().map_err(|e| CmdError {
                    code: None,
                    output: e.to_string(),
                })?;
                let mut out = o.stdout;
                out.extend_from_slice(&o.stderr);
                if o.status.success() {
                    Ok(out)
                } else {
                    Err(CmdError {
                        code: o.status.code(),
                        output: one_line(&out),
                    })
                }
            }),
            run: Arc::new(|n, a| {
                let o = Command::new(n).args(a).output().map_err(|e| CmdError {
                    code: None,
                    output: e.to_string(),
                })?;
                if o.status.success() {
                    return Ok(());
                }
                let mut out = o.stdout;
                out.extend_from_slice(&o.stderr);
                Err(CmdError {
                    code: o.status.code(),
                    output: one_line(&out),
                })
            }),
            reims_qemu: Arc::new(|| Self::reims_qemu_in(&[])),
            extra_kexts: Arc::new(|| {
                let app = AppDir::user().ok_or("no user config folder")?;
                app.load_extra_kexts()
                    .map(|k| k.entries.len())
                    .map_err(|e| e.to_string())
            }),
            settings_file: AppDir::user()
                .map(|a| a.settings_file().to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }

    /// rg-core's Reims QEMU lookup (the `reims_qemu` setting, then each of `reims_dirs`, then
    /// an upstream reims-ctl install). [`Host::system`] passes no `<Workspace>/.reims`, which
    /// lives in the registry; rg-ops sets `reims_qemu` to call this with it.
    pub fn reims_qemu_in(reims_dirs: &[PathBuf]) -> Result<PathBuf, String> {
        let app = AppDir::user();
        let mut roots = reims_dirs.to_vec();
        roots.extend(rg_core::settings::reims_ctl_dir());
        rg_core::settings::find_reims_qemu(
            app.as_ref().and_then(AppDir::reims_qemu_setting).as_deref(),
            &app.map(|a| a.settings_file()).unwrap_or_default(),
            &roots,
        )
        .map_err(|e| e.to_string())
    }

    /// Every host check for the chosen macOS version.
    pub fn checks(&self, version: &str) -> Vec<Check> {
        self.checks_for(&Config {
            version: version.into(),
            ..Config::default()
        })
    }

    /// Every host check for `c`: the base checks, then [`Host::reims_checks_for`]'s.
    pub fn checks_for(&self, c: &Config) -> Vec<Check> {
        let mut out = self.base_checks(&c.version);
        out.extend(self.reims_checks_for(c));
        out
    }

    /// Reims vGPU's checks for `c`, as warnings at most unless `c` runs on Reims vGPU.
    pub fn reims_checks_for(&self, c: &Config) -> Vec<Check> {
        let ram = if c.ram_mib == 0 {
            Config::defaults().ram_mib
        } else {
            c.ram_mib
        };
        let mut out = self.reims_checks(ram);
        for ch in &mut out {
            if ch.status == Status::Fail && !c.reims() {
                ch.status = Status::Warn;
            }
        }
        out
    }

    fn base_checks(&self, version: &str) -> Vec<Check> {
        let flags = self.cpu_flags();
        let mut out = vec![check_virt(flags.as_ref()), self.check_kvm()];
        for (bin, pkg) in TOOLS {
            out.push(if (self.look_path)(bin).is_some() {
                Check::new(bin, Status::Ok, "found", "")
            } else {
                Check::new(
                    bin,
                    Status::Fail,
                    "not found in PATH",
                    format!("sudo apt install {pkg}"),
                )
            });
        }
        out.extend([
            self.check_qemu_version(),
            check_compat(flags.as_ref(), version),
            self.check_msrs(),
            self.check_conflict(),
        ]);
        out
    }

    /// MemTotal from /proc/meminfo in MiB (0 if it cannot be read).
    pub fn mem_total_mib(&self) -> u64 {
        let data = fs::read_to_string(self.proc_root.join("meminfo")).unwrap_or_default();
        data.lines()
            .find_map(|l| {
                let mut f = l.split_whitespace();
                (f.next() == Some("MemTotal:")).then(|| f.next()).flatten()
            })
            .and_then(|kb| kb.parse::<u64>().ok())
            .map_or(0, |kb| kb / 1024)
    }

    /// The `flags` line of /proc/cpuinfo (`None` when it cannot be read).
    pub(crate) fn cpu_flags(&self) -> Option<HashSet<String>> {
        let data = fs::read_to_string(self.proc_root.join("cpuinfo")).ok()?;
        let line = data
            .lines()
            .find_map(|l| l.split_once(':').filter(|(k, _)| k.trim() == "flags"));
        Some(
            line.map(|(_, v)| v.split_whitespace().map(String::from).collect())
                .unwrap_or_default(),
        )
    }

    pub(crate) fn check_kvm(&self) -> Check {
        let name = "/dev/kvm";
        match open_rw(&self.dev_kvm) {
            Ok(()) => Check::new(name, Status::Ok, "read and write allowed", ""),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Check::new(
                name,
                Status::Fail,
                "does not exist",
                "use Set up host (loads kvm_amd or kvm_intel)",
            ),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => Check::new(
                name,
                Status::Fail,
                "permission denied",
                "use Set up host (adds you to the kvm group), then log out and back in",
            ),
            Err(e) => Check::new(name, Status::Fail, e.to_string(), ""),
        }
    }

    fn check_qemu_version(&self) -> Check {
        let name = "QEMU version";
        let hint = "QEMU 8.2.2 or later is required";
        let Some(v) = (self.output)("qemu-system-x86_64", &["--version"])
            .ok()
            .and_then(|o| qemu_version(&o))
        else {
            return Check::new(name, Status::Unknown, "could not read the version", hint);
        };
        let detail = format!("{}.{}.{}", v.0, v.1, v.2);
        if v >= (8, 2, 2) {
            Check::new(name, Status::Ok, detail, "")
        } else {
            Check::new(name, Status::Fail, detail, hint)
        }
    }

    pub(crate) fn msrs_path(&self) -> PathBuf {
        self.sys_root.join("module/kvm/parameters/ignore_msrs")
    }

    pub(crate) fn check_msrs(&self) -> Check {
        let name = "kvm ignore_msrs";
        match fs::read_to_string(self.msrs_path()) {
            Err(_) => Check::new(name, Status::Unknown, "kvm module not loaded", ""),
            Ok(v) if matches!(v.trim(), "Y" | "1") => Check::new(name, Status::Ok, "enabled", ""),
            Ok(_) => Check::new(
                name,
                Status::Fail,
                "disabled: macOS will kernel panic",
                "use Set up host",
            ),
        }
    }

    pub(crate) fn check_conflict(&self) -> Check {
        let name = "Other hypervisors";
        let Ok(entries) = fs::read_dir(&self.proc_root) else {
            return Check::new(name, Status::Unknown, "could not list processes", "");
        };
        let mut found = BTreeSet::new();
        for e in entries.flatten() {
            let pid = e.file_name();
            if pid.to_str().is_none_or(|p| p.parse::<u32>().is_err()) {
                continue;
            }
            let comm = fs::read_to_string(e.path().join("comm")).unwrap_or_default();
            if let Some(h) = hypervisor(comm.trim()) {
                found.insert(h);
            }
        }
        if found.is_empty() {
            return Check::new(
                name,
                Status::Ok,
                "no VM from another hypervisor running",
                "",
            );
        }
        let names: Vec<_> = found.into_iter().collect();
        Check::new(
            name,
            Status::Warn,
            format!("{} VM running", names.join(" and ")),
            "shut that VM down before starting macOS",
        )
    }

    /// Enables ignore_msrs now and at boot via pkexec, then checks the real state.
    pub fn fix_ignore_msrs(&self) -> Result<(), HostError> {
        let res = (self.run)("pkexec", &["/bin/sh", "-c", FIX_SCRIPT]);
        if res.as_ref().is_err_and(pkexec_cancelled) {
            return Err(HostError::Cancelled);
        }
        let runtime_ok = self.check_msrs().status == Status::Ok;
        let conf = fs::read_to_string(self.etc_root.join(MODPROBE_FILE)).unwrap_or_default();
        let persist_ok = conf.contains("ignore_msrs=1");
        match (runtime_ok, persist_ok, res) {
            (true, true, _) => Ok(()),
            (true, false, _) => Err(failed(format!(
                "ignore_msrs is enabled now, but writing /etc/{MODPROBE_FILE} failed (it will not persist after reboot)"
            ))),
            (false, true, _) => Err(failed(format!(
                "/etc/{MODPROBE_FILE} written, but ignore_msrs was not enabled in the running kernel"
            ))),
            (false, false, Err(e)) => Err(failed(format!("pkexec failed: {e}"))),
            (false, false, Ok(())) => Err(failed("the fix was not applied")),
        }
    }

    /// Does, with one password prompt, the basics KVM needs: missing QEMU packages, the KVM
    /// module, kvm group membership (which also opens /dev/udmabuf on Ubuntu), ignore_msrs and
    /// a vm.max_map_count large enough for Reims vGPU. Returns what the user still has to do.
    pub fn setup_host(&self, user: &str) -> Result<String, HostError> {
        if !valid_user_name(user) {
            return Err(failed(format!("unexpected user name {user:?}")));
        }
        let flags = self.cpu_flags().unwrap_or_default();
        let module = if flags.contains("svm") {
            "kvm_amd"
        } else if flags.contains("vmx") {
            "kvm_intel"
        } else {
            return Err(failed(
                "the CPU reports no VT-x/AMD-V: enable virtualization in the BIOS/UEFI first",
            ));
        };
        let pkgs: Vec<&str> = TOOLS
            .iter()
            .filter(|(b, _)| (self.look_path)(b).is_none())
            .map(|(_, p)| *p)
            .collect();
        let pkgs = pkgs.join(" ");
        match (self.run)(
            "pkexec",
            &["/bin/sh", "-c", SETUP_SCRIPT, "sh", user, module, &pkgs],
        ) {
            Err(e) if pkexec_cancelled(&e) => return Err(HostError::Cancelled),
            Err(e) => return Err(failed(format!("host setup failed: {e}"))),
            Ok(()) => {}
        }
        if self.check_msrs().status != Status::Ok {
            return Err(failed("host setup ran, but ignore_msrs is still disabled"));
        }
        if self.check_kvm().status != Status::Ok {
            return Ok(format!(
                "{user} was added to the kvm group: log out and back in (or reboot) so /dev/kvm opens."
            ));
        }
        Ok("The host is ready for KVM.".into())
    }

    /// Runs `script` as root via pkexec with `args` as $1...
    pub(crate) fn run_privileged(&self, script: &str, args: &[&str]) -> Result<(), HostError> {
        let mut all = vec!["/bin/sh", "-c", script, "sh"];
        all.extend_from_slice(args);
        match (self.run)("pkexec", &all) {
            Ok(()) => Ok(()),
            Err(e) if pkexec_cancelled(&e) => Err(HostError::Cancelled),
            Err(e) => Err(failed(e.to_string())),
        }
    }
}

const TOOLS: [(&str, &str); 2] = [
    ("qemu-system-x86_64", "qemu-system-x86"),
    ("qemu-img", "qemu-utils"),
];

fn hypervisor(comm: &str) -> Option<&'static str> {
    match comm {
        "VBoxHeadless" | "VirtualBoxVM" => Some("VirtualBox"),
        "vmware-vmx" => Some("VMware"),
        _ => None,
    }
}

fn check_virt(flags: Option<&HashSet<String>>) -> Check {
    let name = "CPU virtualization";
    match flags {
        None => Check::new(name, Status::Unknown, "could not read /proc/cpuinfo", ""),
        Some(f) if f.contains("vmx") || f.contains("svm") => {
            Check::new(name, Status::Ok, "VT-x/AMD-V present", "")
        }
        Some(_) => Check::new(
            name,
            Status::Fail,
            "VT-x/AMD-V missing",
            "enable virtualization in the BIOS/UEFI",
        ),
    }
}

pub(crate) fn check_compat(flags: Option<&HashSet<String>>, version: &str) -> Check {
    let name = "CPU compatible with version";
    match flags {
        _ if !rg_core::config::profile_for(version).needs_avx2 => {
            Check::new(name, Status::Ok, "this version does not require AVX2", "")
        }
        None => Check::new(name, Status::Unknown, "could not read the CPU flags", ""),
        Some(f) if f.contains("avx2") && f.contains("sse4_1") => {
            Check::new(name, Status::Ok, "SSE4.1 and AVX2 present", "")
        }
        Some(_) => Check::new(
            name,
            Status::Fail,
            "Ventura or later requires SSE4.1 and AVX2",
            "choose a version up to Monterey",
        ),
    }
}

/// The first `version X.Y.Z` in QEMU's `--version` output.
fn qemu_version(out: &[u8]) -> Option<(u32, u32, u32)> {
    let s = String::from_utf8_lossy(out);
    s.match_indices("version ").find_map(|(i, m)| {
        let mut it = s[i + m.len()..].splitn(3, '.');
        let a = it.next()?.parse().ok()?;
        let b = it.next()?.parse().ok()?;
        let c = it.next()?;
        let c = c[..c.find(|ch: char| !ch.is_ascii_digit()).unwrap_or(c.len())]
            .parse()
            .ok()?;
        Some((a, b, c))
    })
}

/// Opens `p` read-write and closes it.
pub(crate) fn open_rw(p: &Path) -> io::Result<()> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(p)
        .map(drop)
}

fn one_line(out: &[u8]) -> String {
    String::from_utf8_lossy(out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn look_path(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(bin))
        .find(|p| fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

/// Tells a dismissed or refused password prompt apart from real errors: pkexec exits 126 when
/// the dialog is dismissed, and the KDE agent makes it exit 127 with "Not authorized"; any
/// other 127 (for example a missing program) stays an error.
pub(crate) fn pkexec_cancelled(e: &CmdError) -> bool {
    e.code == Some(126) || e.code == Some(127) && e.output.contains("Not authorized")
}

/// Go `userNameRe`: `^[a-z_][a-z0-9_.-]*$`.
fn valid_user_name(u: &str) -> bool {
    let mut b = u.bytes();
    b.next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == b'_')
        && b.all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.' | b'-')
        })
}

pub(crate) const MODPROBE_FILE: &str = "modprobe.d/macos-kvm-gui.conf";

/// Constant: nothing from the user's config reaches the root shell.
pub(crate) const FIX_SCRIPT: &str = r"rc=0
echo 1 > /sys/module/kvm/parameters/ignore_msrs || rc=1
printf 'options kvm ignore_msrs=1 report_ignored_msrs=0\n' > /etc/modprobe.d/macos-kvm-gui.conf || rc=1
exit $rc";

/// Constant: user, KVM module and packages arrive as $1, $2 and $3 and are checked again here.
/// Packages are installed only when some are missing.
pub(crate) const SETUP_SCRIPT: &str = r#"case "$1" in ""|-*|*[!a-z0-9_.-]*) echo "invalid user name" >&2; exit 2;; esac
case "$2" in kvm_intel|kvm_amd) ;; *) echo "invalid KVM module" >&2; exit 2;; esac
case "$3" in *[!a-z0-9.+\ -]*) echo "invalid package list" >&2; exit 2;; esac
if [ -n "$3" ]; then
  command -v apt-get >/dev/null || { echo "apt-get not found: install $3 with your package manager" >&2; exit 3; }
  DEBIAN_FRONTEND=noninteractive apt-get install -y $3 || exit 1
fi
[ -e /dev/kvm ] || modprobe "$2" || { echo "could not load $2: enable virtualization in the BIOS" >&2; exit 1; }
getent group kvm >/dev/null || groupadd --system kvm || exit 1
usermod -aG kvm "$1" || exit 1
echo 1 > /sys/module/kvm/parameters/ignore_msrs || exit 1
printf 'options kvm ignore_msrs=1 report_ignored_msrs=0\n' > /etc/modprobe.d/macos-kvm-gui.conf || exit 1
# Reims vGPU maps guest memory in many small views: 2097152 covers a VM of up to 30 GiB.
if [ "$(cat /proc/sys/vm/max_map_count)" -lt 2097152 ]; then
  printf 'vm.max_map_count = 2097152\n' > /etc/sysctl.d/90-macos-kvm-gui.conf || exit 1
  sysctl -qw vm.max_map_count=2097152 || exit 1
fi"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{TestResult, by_name, cmd_err, fake_host, is_root_for, write};

    #[test]
    fn checks_ready_host_are_all_ok() -> TestResult {
        let (_d, h) = fake_host("svm avx2 sse4_1", "Y", &["bash"])?;
        for c in h.checks("sonoma") {
            assert_eq!(c.status, Status::Ok, "{}: {}", c.name, c.detail);
        }
        Ok(())
    }

    #[test]
    fn checks_report_problems() -> TestResult {
        let (_d, mut h) = fake_host("fpu", "N", &["VBoxHeadless"])?;
        h.look_path = Arc::new(|s| (s != "qemu-img").then(|| PathBuf::from(s)));
        h.output = Arc::new(|_, _| Ok(b"QEMU emulator version 6.2.0".to_vec()));
        fs::set_permissions(&h.dev_kvm, fs::Permissions::from_mode(0o000))?;
        let m = by_name(h.checks("sonoma"));
        let mut want = vec![
            ("CPU virtualization", Status::Fail),
            ("qemu-img", Status::Fail),
            ("QEMU version", Status::Fail),
            ("CPU compatible with version", Status::Fail),
            ("kvm ignore_msrs", Status::Fail),
            ("Other hypervisors", Status::Warn),
        ];
        if !is_root_for(&h.dev_kvm) {
            want.push(("/dev/kvm", Status::Fail));
            assert!(m["/dev/kvm"].hint.contains("kvm"), "{:?}", m["/dev/kvm"]);
        }
        for (name, st) in want {
            assert_eq!(m[name].status, st, "{name}: {}", m[name].detail);
        }
        assert!(m["qemu-img"].hint.contains("qemu-utils"));
        assert_eq!(m["Other hypervisors"].detail, "VirtualBox VM running");
        Ok(())
    }

    #[test]
    fn mem_total_is_read_in_mib() -> TestResult {
        let (_d, h) = fake_host("svm", "Y", &[])?;
        write(
            &h.proc_root.join("meminfo"),
            "MemFree:  100 kB\nMemTotal:       195037184 kB\n",
        )?;
        assert_eq!(h.mem_total_mib(), 190466);
        Ok(())
    }

    #[test]
    fn old_version_needs_no_avx2_and_missing_proc_is_inconclusive() -> TestResult {
        let (_d, mut h) = fake_host("vmx", "Y", &[])?;
        assert_eq!(
            check_compat(h.cpu_flags().as_ref(), "monterey").status,
            Status::Ok
        );
        h.proc_root = h.proc_root.join("does-not-exist");
        assert_eq!(h.check_conflict().status, Status::Unknown);
        assert_eq!(check_virt(h.cpu_flags().as_ref()).status, Status::Unknown);
        Ok(())
    }

    #[test]
    fn qemu_version_is_parsed() {
        assert_eq!(
            qemu_version(b"QEMU emulator version 8.2.2 (Debian 1:8.2.2+ds)"),
            Some((8, 2, 2))
        );
        assert_eq!(
            qemu_version(b"QEMU emulator version 10.0.0"),
            Some((10, 0, 0))
        );
        assert_eq!(qemu_version(b"nothing"), None);
    }

    #[test]
    fn fix_ignore_msrs_reports_each_outcome() -> TestResult {
        let (_d, mut h) = fake_host("svm", "N", &[])?;
        let conf = h.etc_root.join(MODPROBE_FILE);
        let (msrs, c2) = (h.msrs_path(), conf.clone());
        h.run = Arc::new(move |name, args| {
            assert!(name == "pkexec" && args[2] == FIX_SCRIPT, "{name} {args:?}");
            write(&msrs, "Y")
                .and_then(|()| write(&c2, "options kvm ignore_msrs=1\n"))
                .map_err(|e| cmd_err(1, &e.to_string()))
        });
        assert_eq!(h.fix_ignore_msrs(), Ok(()));

        write(&h.msrs_path(), "N")?;
        fs::remove_file(&conf)?;
        h.run = Arc::new(|_, _| Err(cmd_err(126, "")));
        assert_eq!(h.fix_ignore_msrs(), Err(HostError::Cancelled));

        let msrs = h.msrs_path();
        h.run = Arc::new(move |_, _| {
            write(&msrs, "Y").ok();
            Err(cmd_err(1, ""))
        });
        assert!(
            h.fix_ignore_msrs()
                .is_err_and(|e| e.to_string().contains("not persist"))
        );

        write(&h.msrs_path(), "N")?;
        h.run = Arc::new(|_, _| Err(cmd_err(127, "")));
        assert!(
            h.fix_ignore_msrs()
                .is_err_and(|e| e.to_string().contains("pkexec failed"))
        );
        Ok(())
    }

    #[test]
    fn setup_host_runs_the_script_with_checked_arguments() -> TestResult {
        let (_d, mut h) = fake_host("svm avx2", "N", &[])?;
        h.look_path = Arc::new(|s| (s != "qemu-img").then(|| PathBuf::from("/usr/bin").join(s)));
        let msrs = h.msrs_path();
        h.run = Arc::new(move |name, a| {
            assert!(
                name == "pkexec"
                    && a[2] == SETUP_SCRIPT
                    && a[4] == "nilton"
                    && a[5] == "kvm_amd"
                    && a[6] == "qemu-utils",
                "{name} {a:?}"
            );
            write(&msrs, "Y").map_err(|e| cmd_err(1, &e.to_string()))
        });
        assert!(h.setup_host("nilton")?.contains("ready"));

        fs::set_permissions(&h.dev_kvm, fs::Permissions::from_mode(0o000))?;
        if !is_root_for(&h.dev_kvm) {
            assert!(h.setup_host("nilton")?.contains("log out"));
        }

        h.run = Arc::new(|_, _| Err(cmd_err(126, "")));
        assert_eq!(h.setup_host("nilton"), Err(HostError::Cancelled));
        h.run =
            Arc::new(|_: &str, _: &[&str]| -> Result<(), CmdError> { panic!("must not elevate") });
        assert!(h.setup_host("bad;user").is_err());

        let (_d2, mut no_virt) = fake_host("fpu", "N", &[])?;
        no_virt.run = h.run.clone();
        assert!(
            no_virt
                .setup_host("nilton")
                .is_err_and(|e| e.to_string().contains("BIOS"))
        );
        Ok(())
    }

    /// The real shell must reject hostile arguments before running anything.
    #[test]
    fn setup_script_rejects_injection() -> TestResult {
        for args in [
            ["-o", "kvm_amd", ""],
            ["nilton;reboot", "kvm_amd", ""],
            ["nilton", "kvm_amd;reboot", ""],
            ["nilton", "kvm_amd", "qemu-utils;reboot"],
            ["nilton", "kvm_amd", "$(id)"],
        ] {
            let st = Command::new("/bin/sh")
                .args(["-c", SETUP_SCRIPT, "sh"])
                .args(args)
                .output()?
                .status;
            assert_eq!(st.code(), Some(2), "{args:?}");
        }
        Ok(())
    }

    #[test]
    fn pkexec_cancel_is_told_apart() -> TestResult {
        let kde = cmd_err(
            127,
            "Error executing command as another user: Not authorized This incident has been reported.",
        );
        let missing = cmd_err(
            127,
            "Error executing command as another user: /x: No such file or directory",
        );
        assert!(pkexec_cancelled(&cmd_err(126, "")));
        assert!(pkexec_cancelled(&kde));
        assert!(!pkexec_cancelled(&missing));
        assert!(!pkexec_cancelled(&cmd_err(1, "")));

        let (_d, mut h) = fake_host("svm", "N", &[])?;
        h.run = Arc::new(move |_, _| Err(kde.clone()));
        assert_eq!(h.setup_host("nilton"), Err(HostError::Cancelled));
        Ok(())
    }

    #[test]
    fn user_names_follow_the_go_pattern() {
        assert!(valid_user_name("nilton") && valid_user_name("_a.b-1"));
        assert!(
            !valid_user_name("")
                && !valid_user_name("1a")
                && !valid_user_name("Nilton")
                && !valid_user_name("a;b")
        );
    }
}

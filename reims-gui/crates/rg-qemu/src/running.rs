//! Finding the QEMU that uses a VM's disk, whoever started it (Go `running.go`).

use crate::{Error, Result};
use rg_core::Config;
use rg_core::workspace::{DISK_FILE, real_path};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The prefix of the private run directories (QMP, control and VNC sockets).
pub const RUN_DIR_PREFIX: &str = "macos-kvm-gui-";

/// A QEMU process that uses a VM's disk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunInfo {
    pub pid: u32,
    /// When the process started (`/proc/<pid>`'s time).
    pub started: Option<SystemTime>,
    /// Started by this app (or the Go app): its QMP socket is in a run directory.
    pub by_app: bool,
    /// The VNC socket, when there is one.
    pub vnc_socket: Option<PathBuf>,
    /// The second QMP socket, for a process other than the starter.
    pub control: Option<PathBuf>,
    /// The offline installer disc is attached.
    pub disc: bool,
    /// It runs on Reims vGPU (apple-gfx-pci), whatever the VM's config says now.
    pub reims: bool,
}

impl RunInfo {
    /// How long the VM has been running, to the second.
    pub fn uptime(&self) -> Duration {
        let d = self
            .started
            .and_then(|s| s.elapsed().ok())
            .unwrap_or_default();
        Duration::from_secs(d.as_secs() + u64::from(d.subsec_millis() >= 500))
    }
}

/// Finds the QEMU process using `c`'s disk, including one started outside the app (for example
/// by OSX-KVM's OpenCore-Boot.sh, which uses a path relative to its folder).
pub fn find_running(c: &Config) -> Option<RunInfo> {
    running_in(Path::new("/proc"), c)
}

/// [`find_running`] over another `/proc` (tests).
pub fn running_in(proc: &Path, c: &Config) -> Option<RunInfo> {
    let disk = real_path(&c.workspace_dir().ok()?.join(DISK_FILE));
    for e in std::fs::read_dir(proc).ok()?.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        let base = e.path();
        let comm = std::fs::read(base.join("comm")).unwrap_or_default();
        if !comm.starts_with(b"qemu-system") {
            continue;
        }
        let Ok(raw) = std::fs::read(base.join("cmdline")) else {
            continue;
        };
        let raw = String::from_utf8_lossy(&raw);
        let args: Vec<&str> = raw.trim_end_matches('\0').split('\0').collect();
        let cwd = std::fs::read_link(base.join("cwd")).ok();
        if !uses_disk(&args, &disk, cwd.as_deref()) {
            continue;
        }
        let mut info = RunInfo {
            pid,
            started: std::fs::metadata(&base).and_then(|m| m.modified()).ok(),
            ..RunInfo::default()
        };
        for (i, a) in args.iter().enumerate() {
            let next = args.get(i + 1).copied().unwrap_or("");
            if *a == "-qmp" && next.contains(RUN_DIR_PREFIX) {
                info.by_app = true;
            }
            if *a == "-qmp" && next.contains("qmp-control.sock") {
                info.control = Some(socket_path(next, ",server"));
            }
            if a.starts_with("id=InstallerDisc,") {
                info.disc = true;
            }
            if a.starts_with("apple-gfx-pci") {
                info.reims = true;
            }
            if *a == "-vnc" && i + 1 < args.len() {
                info.vnc_socket = Some(socket_path(next, ",lock-key-sync"));
            }
        }
        return Some(info);
    }
    None
}

/// `unix:<path>,<opts>` → path, with QEMU's ",," unescaped.
fn socket_path(arg: &str, opts: &str) -> PathBuf {
    let s = arg.strip_prefix("unix:").unwrap_or(arg);
    let s = s.split_once(opts).map_or(s, |(p, _)| p);
    PathBuf::from(s.replace(",,", ","))
}

/// Whether a QEMU command line opens `disk`, written absolute (with QEMU's ",," comma escaping)
/// or relative to the process's working directory.
fn uses_disk(args: &[&str], disk: &Path, cwd: Option<&Path>) -> bool {
    args.iter().any(|a| {
        a.replace(",,", "\x01").split(',').any(|opt| {
            let Some(f) = opt.strip_prefix("file=") else {
                return false;
            };
            let f = f.replace('\x01', ",");
            let f = f.trim_matches('"');
            let p = match cwd {
                // Not join-and-clean: ".." must apply after symlinks.
                Some(cwd) if !f.starts_with('/') => PathBuf::from(format!("{}/{f}", cwd.display())),
                _ => PathBuf::from(f),
            };
            real_path(&p) == disk
        })
    })
}

/// Accepts a socket only inside a private run directory this user owns (0700), as the app
/// creates them, so a process never drives a socket another user could have placed.
pub fn check_run_dir(sock: &Path) -> Result<()> {
    let dir = sock.parent().unwrap_or(Path::new(""));
    let named = dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with(RUN_DIR_PREFIX));
    if !named {
        return Err(Error::msg(format!(
            "{} is not in a run directory of this app",
            sock.display()
        )));
    }
    let uid = rustix::process::getuid().as_raw();
    let st = std::fs::symlink_metadata(dir)?;
    if !st.is_dir() || st.permissions().mode() & 0o777 != 0o700 || st.uid() != uid {
        return Err(Error::msg(format!(
            "{} is not a private directory of this user",
            dir.display()
        )));
    }
    // The socket itself, not what a symlink would point to.
    let st = std::fs::symlink_metadata(sock)?;
    if !st.file_type().is_socket() || st.uid() != uid {
        return Err(Error::msg(format!(
            "{} is not a socket of this user",
            sock.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Result as IoResult;

    fn fake_proc(
        proc: &Path,
        pid: &str,
        comm: &str,
        cwd: Option<&str>,
        args: &[&str],
    ) -> IoResult<()> {
        let d = proc.join(pid);
        std::fs::create_dir_all(&d)?;
        std::fs::write(d.join("comm"), format!("{comm}\n"))?;
        std::fs::write(d.join("cmdline"), format!("{}\0", args.join("\0")))?;
        if let Some(cwd) = cwd {
            std::os::unix::fs::symlink(cwd, d.join("cwd"))?;
        }
        Ok(())
    }

    fn config(workspace: &str) -> Config {
        Config {
            workspace: workspace.into(),
            ..Config::defaults()
        }
    }

    #[test]
    fn running_detection() -> IoResult<()> {
        let t = tempfile::tempdir()?;
        let proc = t.path();
        let c = config("/vm/a,b");
        fake_proc(
            proc,
            "100",
            "bash",
            None,
            &["bash", "-c", "file=/vm/a,,b/mac_hdd_ng.img"],
        )?;
        assert_eq!(running_in(proc, &c), None, "only qemu processes count");
        fake_proc(
            proc,
            "200",
            "qemu-system-x86",
            None,
            &[
                "qemu-system-x86_64",
                "-drive",
                "id=MacHDD,if=none,format=qcow2,file=/vm/a,,b/mac_hdd_ng.img",
                "-vnc",
                "unix:/run/user/1000/macos-kvm-gui-1/vnc.sock,lock-key-sync=off",
                "-qmp",
                "unix:/run/user/1000/macos-kvm-gui-1/qmp.sock,server=on,wait=off",
                "-qmp",
                "unix:/run/user/1000/macos-kvm-gui-1/qmp-control.sock,server=on,wait=off",
                "-drive",
                "id=InstallerDisc,if=none,format=raw,snapshot=on,file=/c/InstallAssistant.iso",
            ],
        )?;
        let r = running_in(proc, &c).ok_or_else(|| std::io::Error::other("not found"))?;
        assert_eq!(r.pid, 200);
        assert!(r.by_app && r.disc && !r.reims && r.started.is_some());
        assert_eq!(
            r.vnc_socket.as_deref(),
            Some(Path::new("/run/user/1000/macos-kvm-gui-1/vnc.sock"))
        );
        assert_eq!(
            r.control.as_deref(),
            Some(Path::new("/run/user/1000/macos-kvm-gui-1/qmp-control.sock"))
        );

        // The running process tells whether it is on Reims vGPU, not the config.
        std::fs::remove_dir_all(proc.join("200"))?;
        fake_proc(
            proc,
            "201",
            "qemu-system-x86",
            None,
            &[
                "qemu-system-x86_64",
                "-drive",
                "file=/vm/a,,b/mac_hdd_ng.img",
                "-device",
                "apple-gfx-pci,id=gfx,displays=1,resolution=1920x1080",
            ],
        )?;
        assert!(running_in(proc, &c).is_some_and(|r| r.reims && !r.by_app));
        Ok(())
    }

    #[test]
    fn finds_osx_kvm_relative_quoted_path() -> IoResult<()> {
        let t = tempfile::tempdir()?;
        fake_proc(
            t.path(),
            "300",
            "qemu-system-x86",
            Some("/home/me/OSX-KVM"),
            &[
                "qemu-system-x86_64",
                "-drive",
                r#"id=MacHDD,if=none,file="./mac_hdd_ng.img",format=qcow2"#,
            ],
        )?;
        assert!(running_in(t.path(), &config("/home/me/OSX-KVM")).is_some_and(|r| !r.by_app));
        assert_eq!(running_in(t.path(), &config("/elsewhere")), None);
        Ok(())
    }

    #[test]
    fn check_run_dir_accepts_only_private_dirs() -> IoResult<()> {
        let t = tempfile::tempdir()?;
        let dir = t.path().join("macos-kvm-gui-1");
        std::fs::create_dir(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        let sock = dir.join("vnc.sock");
        let _l = std::os::unix::net::UnixListener::bind(&sock)?;
        assert!(check_run_dir(&sock).is_ok());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))?;
        assert!(check_run_dir(&sock).is_err(), "group/world-readable dir");
        assert!(
            check_run_dir(&t.path().join("vnc.sock")).is_err(),
            "not a run dir"
        );
        Ok(())
    }
}

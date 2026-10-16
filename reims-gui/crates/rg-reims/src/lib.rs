//! Reims vGPU packages: manifest, install, Docker build, activation and pruning (port of the Go
//! app's `internal/core/reims.go` and `reimspkg.go`).
//!
//! Reims vGPU (github.com/steelbrain/reims-vgpu) runs on its own QEMU build, which the app
//! installs from a prebuilt package (pinned by the embedded manifest) or builds in Docker (the
//! embedded Dockerfile), but never embeds. Both land in `<ReimsDir>/qemu/<build>`, and
//! `qemu/current` is switched to a build only once it works on this host. See `API.md`.

mod activate;
mod docker;
mod install;
mod process;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use rg_core::AppDir;
use rg_core::settings::find_reims_qemu;
use serde::Deserialize;

/// The Ubuntu releases packages and builds exist for.
pub const RELEASES: [&str; 2] = ["24.04", "26.04"];

/// The Dockerfile that builds upstream's QEMU at a pinned commit.
pub const DOCKERFILE: &str = include_str!("../reims/Dockerfile");
const MANIFEST_JSON: &str = include_str!("../reims/manifest.json");

/// Where the manifest's tag is published.
const RELEASE_URL: &str = "https://github.com/jniltinho/macos-kvm-gui/releases/download/";
/// Package limits: a size over a limit fails the install, nothing is ever truncated.
const MAX_PACKAGE: u64 = 512 << 20;
const MAX_ENTRIES: usize = 20000;
const MAX_EXPANDED: u64 = 2 << 30;

/// An install, build or status failure, with a message meant for the user.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Msg(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

fn msg(s: impl Into<String>) -> Error {
    Error::Msg(s.into())
}

/// Receives each progress line; it may be called from another thread.
pub type Log<'a> = &'a (dyn Fn(&str) + Sync);

/// One release's prebuilt package.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Package {
    pub file: String,
    pub sha256: String,
}

/// The embedded `manifest.json`: the Releases tag and the package of each Ubuntu release.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub tag: String,
    pub commit: String,
    pub packages: std::collections::BTreeMap<String, Package>,
}

/// The embedded manifest.
pub fn manifest() -> Result<Manifest> {
    serde_json::from_str(MANIFEST_JSON).map_err(|e| msg(format!("the embedded manifest: {e}")))
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The Reims QEMU in use (`ReimsInfo`).
#[derive(Debug)]
pub struct Info {
    /// The binary [`Reims::qemu`] finds.
    pub qemu: PathBuf,
    /// Its build folder's name.
    pub build: String,
    /// Upstream's commit, from the folder's name.
    pub commit: String,
    /// `Ok` when it lists apple-gfx-pci.
    pub apple_gfx: Result<()>,
}

/// Reims packages under one `<Workspace>/.reims`. The other fields are the host's paths and the
/// download source; tests point them at fakes.
#[derive(Debug, Clone)]
pub struct Reims {
    dir: PathBuf,
    os_release: PathBuf,
    proc_root: PathBuf,
    ldd: String,
    release_url: String,
    manifest_json: String,
    max_expanded: u64,
    /// DER root certificates trusted instead of the web's (tests' local server).
    roots: Option<Vec<Vec<u8>>>,
    /// `reims_qemu` / `MKG_REIMS_QEMU`, absolute.
    qemu_setting: Option<PathBuf>,
    /// `$XDG_DATA_HOME`, for upstream reims-ctl's install.
    data_dir: Option<PathBuf>,
}

impl Reims {
    /// Reims packages in `dir` (`<Workspace>/.reims`, see `rg_core::config::reims_dir`). Reads the
    /// `reims_qemu` setting and `$XDG_DATA_HOME` now.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            os_release: "/etc/os-release".into(),
            proc_root: "/proc".into(),
            ldd: "ldd".into(),
            release_url: RELEASE_URL.into(),
            manifest_json: MANIFEST_JSON.into(),
            max_expanded: MAX_EXPANDED,
            roots: None,
            qemu_setting: AppDir::user().and_then(|a| a.reims_qemu_setting()),
            data_dir: dirs::data_dir(),
        }
    }

    /// `<Workspace>/.reims`.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The host's Ubuntu release when it is one of [`RELEASES`].
    pub fn host_ubuntu_release(&self) -> Result<String> {
        let data = fs::read_to_string(&self.os_release)?;
        let kv: std::collections::HashMap<&str, &str> = data
            .lines()
            .filter_map(|l| l.trim().split_once('='))
            .map(|(k, v)| (k, v.trim_matches(['"', '\''])))
            .collect();
        let version = kv.get("VERSION_ID").copied().unwrap_or_default();
        if kv.get("ID") == Some(&"ubuntu") && RELEASES.contains(&version) {
            return Ok(version.to_string());
        }
        let name = match kv.get("PRETTY_NAME") {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => format!("not identified by {}", self.os_release.display()),
        };
        Err(msg(format!(
            "Reims vGPU supports Ubuntu {} hosts, and this one is {name}",
            RELEASES.join(" and ")
        )))
    }

    /// Finds the Reims QEMU: the `reims_qemu` setting, the app's own install in
    /// `<Workspace>/.reims`, then an upstream reims-ctl install.
    pub fn qemu(&self) -> Result<PathBuf> {
        let mut roots = vec![self.dir.clone()];
        roots.extend(self.data_dir.as_ref().map(|d| d.join("reims-ctl")));
        let file = AppDir::user()
            .map(|a| a.settings_file())
            .unwrap_or_default();
        find_reims_qemu(self.qemu_setting.as_deref(), &file, &roots).map_err(|e| msg(e.to_string()))
    }

    /// Describes the Reims QEMU a reims-vgpu VM would run.
    pub fn installed(&self) -> Result<Info> {
        let qemu = self.qemu()?;
        let apple_gfx = activate::probe_apple_gfx(&qemu);
        let build = fs::canonicalize(&qemu)
            .ok()
            .and_then(|real| {
                Some(
                    real.parent()?
                        .parent()?
                        .file_name()?
                        .to_string_lossy()
                        .into_owned(),
                )
            })
            .unwrap_or_default();
        let commit = if build.is_empty() {
            String::new()
        } else {
            reims_commit(&build).to_string()
        };
        Ok(Info {
            qemu,
            build,
            commit,
            apple_gfx,
        })
    }

    /// Downloads the package for the host's release, checks it against the manifest, unpacks it
    /// and activates it. Setting `cancel` stops it.
    pub fn install(&self, cancel: &AtomicBool, log: Log<'_>) -> Result<()> {
        install::install(self, cancel, log)
    }

    /// Builds upstream's QEMU at the Dockerfile's commit in an `ubuntu:<release>` container
    /// (`None` = the host's release), caching the build in `<ReimsDir>/build-<release>`, then
    /// stages and activates it as [`Reims::install`] does. Setting `cancel` interrupts Docker.
    pub fn build(&self, release: Option<&str>, cancel: &AtomicBool, log: Log<'_>) -> Result<()> {
        docker::build(self, release, cancel, log)
    }

    fn manifest(&self) -> Result<Manifest> {
        serde_json::from_str(&self.manifest_json).map_err(|e| msg(format!("the manifest: {e}")))
    }

    /// Takes the host-wide lock held through a whole install or build, including the switch of
    /// current, and returns `<ReimsDir>/qemu` with the lock (released when dropped).
    fn lock(&self, cancel: &AtomicBool) -> Result<(PathBuf, fs::File)> {
        let dir = std::path::absolute(&self.dir)?;
        let qdir = dir.join("qemu");
        fs::create_dir_all(&qdir)?;
        let f = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(".lock"))?;
        loop {
            match f.try_lock() {
                Ok(()) => break,
                Err(fs::TryLockError::WouldBlock) => {}
                Err(fs::TryLockError::Error(e)) => return Err(e.into()),
            }
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        // Leftovers of an install that died while it held the lock.
        for e in fs::read_dir(&qdir)?.flatten() {
            if e.file_name().to_string_lossy().starts_with(".tmp-") {
                let _ = remove_all(&e.path());
            }
        }
        Ok((qdir, f))
    }
}

/// Removes a file, a link or a folder tree.
fn remove_all(p: &Path) -> io::Result<()> {
    if fs::symlink_metadata(p)?.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
}

/// Upstream's commit from a build name: `<tag or date-time>-<commit>`, with `-ubuntu<release>`
/// after it for the app's builds.
pub fn reims_commit(build: &str) -> &str {
    let build = match build.rfind("-ubuntu") {
        Some(i) if i > 0 => &build[..i],
        _ => build,
    };
    build.rsplit('-').next().unwrap_or(build)
}

#[cfg(test)]
pub(crate) mod testutil;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[test]
    fn host_ubuntu_release_accepts_only_supported_ubuntu() -> TestResult {
        let cases = [
            (
                "NAME=\"Ubuntu\"\nID=ubuntu\nVERSION_ID=\"24.04\"\n",
                "24.04",
            ),
            ("ID=ubuntu\nVERSION_ID=\"26.04\"\n", "26.04"),
            (
                "ID=ubuntu\nVERSION_ID=\"22.04\"\nPRETTY_NAME=\"Ubuntu 22.04.5 LTS\"\n",
                "",
            ),
            (
                "ID=debian\nVERSION_ID=\"13\"\nPRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"",
                "",
            ),
        ];
        for (body, want) in cases {
            let t = TempDir::new()?;
            let r = reims_in(&t, body)?;
            match r.host_ubuntu_release() {
                Ok(got) => assert_eq!(got, want, "{body}"),
                Err(e) => {
                    assert_eq!(want, "", "{body}: {e}");
                    assert!(e.to_string().contains("24.04 and 26.04"), "{e}");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn manifest_matches_the_dockerfile_commit() -> TestResult {
        let m = manifest()?;
        let pinned = DOCKERFILE
            .lines()
            .find_map(|l| l.strip_prefix("ARG REIMS_COMMIT="))
            .ok_or("no REIMS_COMMIT")?;
        assert_eq!(pinned, m.commit);
        assert!(m.commit.len() == 40 && m.commit.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(m.tag, format!("reims-{}", &m.commit[..7]));
        for rel in RELEASES {
            let p = m.packages.get(rel).ok_or(rel)?;
            assert_eq!(
                p.file,
                format!("reims-qemu-{}-ubuntu{rel}-x86_64.tar.gz", &m.commit[..7])
            );
            assert!(is_sha256(&p.sha256), "{rel}: {p:?}");
        }
        Ok(())
    }

    #[test]
    fn reims_commit_reads_the_build_name() {
        for (name, want) in [
            ("20261016-212623-c099dd5a-ubuntu24.04", "c099dd5a"),
            ("reims-c099dd5-ubuntu26.04", "c099dd5"),
            ("20261016-212623-c099dd5a", "c099dd5a"), // upstream's own install
        ] {
            assert_eq!(reims_commit(name), want);
        }
    }

    #[test]
    fn dockerfile_chowns_on_exit() {
        // A failed build must not leave root-owned files in the Workspace: the chown is an EXIT
        // trap set before anything is written to /build, not a last step.
        let trap =
            DOCKERFILE.find(r#"trap 'chown -R "${HOST_UID:-0}:${HOST_GID:-0}" /build' EXIT"#);
        let build = DOCKERFILE.find("mkdir -p /build");
        assert!(
            matches!((trap, build), (Some(t), Some(b)) if t < b),
            "{trap:?} {build:?}"
        );
    }

    #[test]
    fn qemu_prefers_the_setting_then_the_workspace_then_reims_ctl() -> TestResult {
        let t = TempDir::new()?;
        let mut r = reims_in(&t, "")?;
        let data = t.path().join("data");
        r.data_dir = Some(data.clone());
        let err = r.qemu().err().ok_or("found a QEMU")?.to_string();
        assert!(
            err.contains("not installed") && err.contains(".reims/qemu/current/bin"),
            "{err}"
        );

        let upstream = data.join("reims-ctl/qemu/current/bin/qemu-system-x86_64");
        write(&upstream, "")?;
        assert_eq!(r.qemu()?, upstream);
        let own = r.dir.join("qemu/current/bin/qemu-system-x86_64");
        write(&own, "")?;
        assert_eq!(r.qemu()?, own);

        r.qemu_setting = Some(t.path().join("missing"));
        assert!(
            r.qemu()
                .is_err_and(|e| e.to_string().contains("reims_qemu in"))
        );
        r.qemu_setting = Some(upstream.clone());
        assert_eq!(r.qemu()?, upstream);
        Ok(())
    }

    #[test]
    fn lock_waits_until_cancelled() -> TestResult {
        let t = TempDir::new()?;
        let r = reims_in(&t, "")?;
        let (qdir, _held) = r.lock(&AtomicBool::new(false))?;
        assert!(qdir.ends_with("qemu"));
        assert!(matches!(
            r.lock(&AtomicBool::new(true)),
            Err(Error::Cancelled)
        ));
        Ok(())
    }
}

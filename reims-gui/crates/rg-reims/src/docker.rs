//! Building Reims vGPU's QEMU in Docker with the embedded Dockerfile.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use crate::{DOCKERFILE, Error, Log, RELEASES, Reims, Result, activate, msg, process};

const DOCKER_GROUP: &str =
    "add yourself to the docker group (sudo usermod -aG docker $USER, then log in again)";

pub(crate) fn build(
    r: &Reims,
    release: Option<&str>,
    cancel: &AtomicBool,
    log: Log<'_>,
) -> Result<()> {
    let release = match release {
        None | Some("") => match r.host_platform() {
            Ok(rel) if RELEASES.contains(&rel.as_str()) => rel,
            Ok(rel) => {
                return Err(msg(format!(
                    "Reims vGPU builds from source for Ubuntu {} only; this host is {rel}: install its prebuilt package instead (Install Reims vGPU, or `reims install`), or build an Ubuntu one with --release",
                    RELEASES.join(" and ")
                )));
            }
            Err(e) => return Err(msg(format!("{e}: build for one of them with --release"))),
        },
        Some(rel) if RELEASES.contains(&rel) => rel.to_string(),
        Some(rel) => {
            return Err(msg(format!(
                "Reims vGPU builds for Ubuntu {}, not {rel:?}",
                RELEASES.join(" and ")
            )));
        }
    };
    let mut info = String::new();
    match process::run_lines(docker(["info"]), None, cancel, None, &mut |l| {
        info = l.to_string()
    }) {
        Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
            return Err(msg(format!(
                "the build needs Docker: install it (sudo apt install docker.io) and {DOCKER_GROUP}"
            )));
        }
        Ok(st) if !st.success() => {
            return Err(msg(format!(
                "Docker is not usable ({info}): start it (sudo systemctl start docker) and {DOCKER_GROUP}"
            )));
        }
        other => {
            other?;
        }
    }
    let (qdir, _lock) = r.lock(cancel)?;
    let work = qdir
        .parent()
        .ok_or_else(|| msg("no parent folder"))?
        .join(format!("build-{release}"));
    let share = work.join("share/reims-ctl/qemu");
    let _ = fs::remove_dir_all(&share); // only this run's build may be found there
    fs::create_dir_all(&work)?;
    let image = format!("macos-kvm-gui-reims:{release}");
    log(&format!("Preparing the build image {image}"));
    let ubuntu = format!("UBUNTU={release}");
    let cmd = docker([
        "build",
        "--progress=plain",
        "--build-arg",
        &ubuntu,
        "-t",
        &image,
        "-",
    ]);
    run_logged(cmd, Some(DOCKERFILE.as_bytes()), cancel, log)
        .map_err(|e| context("docker build", e))?;
    log(&format!(
        "Building Reims vGPU for Ubuntu {release} (about 10 minutes the first time)"
    ));
    let me = fs::metadata("/proc/self")?; // owned by this process's uid and gid
    let (uid, gid) = (
        format!("HOST_UID={}", me.uid()),
        format!("HOST_GID={}", me.gid()),
    );
    let mount = format!("{}:/build", work.display());
    let cmd = docker(["run", "--rm", "-e", &uid, "-e", &gid, "-v", &mount, &image]);
    run_logged(cmd, None, cancel, log).map_err(|e| context("the build", e))?;
    let built = fs::canonicalize(share.join("current"))
        .map_err(|e| msg(format!("the build left no QEMU: {e}")))?;
    let base = built
        .file_name()
        .ok_or_else(|| msg("the build left no QEMU"))?;
    let name = format!("{}-ubuntu{release}", base.to_string_lossy());
    fs::rename(&built, qdir.join(&name))?;
    activate::activate(r, &qdir, &name, &release, log)
}

fn docker<const N: usize>(args: [&str; N]) -> Command {
    let mut c = Command::new("docker");
    c.args(args).env("DOCKER_BUILDKIT", "1");
    c
}

fn context(what: &str, e: Error) -> Error {
    match e {
        Error::Cancelled => e,
        e => msg(format!("{what}: {e}")),
    }
}

/// Runs a command with each output line sent to log; a failure is an error.
fn run_logged(cmd: Command, stdin: Option<&[u8]>, cancel: &AtomicBool, log: Log<'_>) -> Result<()> {
    let st = process::run_lines(cmd, stdin, cancel, None, &mut |l| log(l))?;
    if st.success() {
        Ok(())
    } else {
        Err(msg(st.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[test]
    fn build_checks_the_release_before_docker() -> TestResult {
        let t = TempDir::new()?;
        let r = reims_in(
            &t,
            "ID=ubuntu\nVERSION_ID=\"22.04\"\nPRETTY_NAME=\"Ubuntu 22.04.5 LTS\"\n",
        )?;
        let no = AtomicBool::new(false);
        let log = |_: &str| {};
        let err = r
            .build(Some("22.04"), &no, &log)
            .err()
            .ok_or("built 22.04")?
            .to_string();
        assert!(err.contains("24.04 and 26.04"), "{err}");
        let err = r
            .build(None, &no, &log)
            .err()
            .ok_or("built on 22.04")?
            .to_string();
        assert!(
            err.contains("--release"),
            "an unsupported host stops before Docker: {err}"
        );
        Ok(())
    }
}

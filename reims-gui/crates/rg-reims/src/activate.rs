//! Activation of a staged build (host checks, atomic `current` switch) and pruning.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::{Log, Reims, Result, msg, process};

/// Switches `qemu/current` to the build `name` when it is for the host's release and works here;
/// a build for another release stays staged. Older builds of the same release go, unless a
/// running QEMU uses them.
pub(crate) fn activate(r: &Reims, qdir: &Path, name: &str, rel: &str, log: Log<'_>) -> Result<()> {
    let res = switch(r, qdir, name, rel, log);
    prune(r, qdir, rel, name, log);
    res
}

fn switch(r: &Reims, qdir: &Path, name: &str, rel: &str, log: Log<'_>) -> Result<()> {
    let dir = qdir.join(name);
    if r.host_ubuntu_release().ok().as_deref() != Some(rel) {
        log(&format!(
            "{} is for Ubuntu {rel}, not this host: it stays staged, not activated",
            dir.display()
        ));
        return Ok(());
    }
    check_build(r, &dir).map_err(|e| {
        msg(format!(
            "{} stays staged, not activated: {e}",
            dir.display()
        ))
    })?;
    let tmp = qdir.join(".tmp-current");
    let _ = fs::remove_file(&tmp);
    std::os::unix::fs::symlink(name, &tmp)?;
    let current = qdir.join("current");
    if let Err(e) = fs::rename(&tmp, &current) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    log(&format!(
        "Reims vGPU {name} is active: {}",
        current.display()
    ));
    Ok(())
}

/// Runs a build's QEMU on this host: every library resolves, apple-gfx-pci is listed, and its
/// option ROM is there.
fn check_build(r: &Reims, dir: &Path) -> Result<()> {
    if fs::metadata(dir.join("share/qemu/ReimsVgpu.rom")).is_err() {
        return Err(msg("it has no share/qemu/ReimsVgpu.rom"));
    }
    let bin = dir.join("bin/qemu-system-x86_64");
    let out = Command::new(&r.ldd)
        .arg(&bin)
        .output()
        .map_err(|e| msg(format!("{}: {e}", r.ldd)))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let missing = ldd_missing(&text);
    if !missing.is_empty() {
        return Err(missing_libs_error(&missing));
    }
    if !out.status.success() {
        return Err(msg(format!(
            "ldd {}: {}: {}",
            bin.display(),
            out.status,
            text.trim()
        )));
    }
    probe_apple_gfx(&bin)
}

/// Checks that a QEMU binary lists the Reims vGPU device.
pub(crate) fn probe_apple_gfx(bin: &Path) -> Result<()> {
    let mut out = String::new();
    let mut cmd = Command::new(bin);
    cmd.args(["-device", "help"]);
    let never = AtomicBool::new(false);
    let status = process::run_lines(cmd, None, &never, Some(Duration::from_secs(30)), &mut |l| {
        out.push_str(l);
        out.push('\n');
    })
    .map_err(|e| msg(format!("{} -device help: {e}", bin.display())))?;
    if !status.success() {
        let last = out.trim().lines().last().unwrap_or_default();
        return Err(msg(format!(
            "{} -device help: {status}: {last}",
            bin.display()
        )));
    }
    if !out.contains("\"apple-gfx-pci\"") {
        return Err(msg(format!(
            "{} does not list apple-gfx-pci: it is not a Reims vGPU build",
            bin.display()
        )));
    }
    Ok(())
}

/// The libraries ldd could not find.
fn ldd_missing(out: &str) -> Vec<&str> {
    out.lines()
        .filter_map(|l| l.trim().strip_suffix(" => not found"))
        .collect()
}

/// The libraries a Reims QEMU needs beyond a bare ubuntu image and the packages that ship them,
/// the same on 24.04 and 26.04 (measured with ldd in both images).
const LIB_PACKAGES: [(&str, &str); 20] = [
    ("libslirp.so.0", "libslirp0"),
    ("libpixman-1.so.0", "libpixman-1-0"),
    ("libfdt.so.1", "libfdt1"),
    ("libglib-2.0.so.0", "libglib2.0-0t64"),
    ("libgio-2.0.so.0", "libglib2.0-0t64"),
    ("libgobject-2.0.so.0", "libglib2.0-0t64"),
    ("libgmodule-2.0.so.0", "libglib2.0-0t64"),
    ("libgtk-3.so.0", "libgtk-3-0t64"),
    ("libgdk-3.so.0", "libgtk-3-0t64"),
    ("libgdk_pixbuf-2.0.so.0", "libgdk-pixbuf-2.0-0"),
    ("libcairo.so.2", "libcairo2"),
    ("libepoxy.so.0", "libepoxy0"),
    ("libpng16.so.16", "libpng16-16t64"),
    ("libjpeg.so.8", "libjpeg-turbo8"),
    ("libX11.so.6", "libx11-6"),
    ("libgbm.so.1", "libgbm1"),
    ("libvulkan.so.1", "libvulkan1"),
    ("libpipewire-0.3.so.0", "libpipewire-0.3-0t64"),
    ("libpulse.so.0", "libpulse0"),
    ("libasound.so.2", "libasound2t64"),
];

fn missing_libs_error(missing: &[&str]) -> crate::Error {
    let mut named = Vec::new();
    let mut pkgs: Vec<&str> = Vec::new();
    for lib in missing {
        match LIB_PACKAGES.iter().find(|(l, _)| l == lib) {
            Some((_, p)) => {
                named.push(format!("{lib} ({p})"));
                if !pkgs.contains(p) {
                    pkgs.push(p);
                }
            }
            None => named.push(lib.to_string()),
        }
    }
    let mut m = format!(
        "this host lacks libraries its QEMU needs: {}",
        named.join(", ")
    );
    if !pkgs.is_empty() {
        m += &format!("; install them with: sudo apt install {}", pkgs.join(" "));
    }
    msg(m)
}

/// Removes the builds of release `rel` other than `keep` and current's, sparing any a running
/// QEMU uses.
fn prune(r: &Reims, qdir: &Path, rel: &str, keep: &str, log: Log<'_>) {
    let cur = fs::read_link(qdir.join("current")).unwrap_or_default();
    let Ok(ents) = fs::read_dir(qdir) else { return };
    let suffix = format!("-ubuntu{rel}");
    for e in ents.flatten() {
        let n = e.file_name();
        let name = n.to_string_lossy();
        if !e.file_type().is_ok_and(|t| t.is_dir())
            || name == keep
            || cur.as_os_str() == n
            || !name.ends_with(&suffix)
        {
            continue;
        }
        let d = e.path();
        if in_use(&r.proc_root, &d) {
            log(&format!("{} is kept: a running VM uses it", d.display()));
        } else if fs::remove_dir_all(&d).is_ok() {
            log(&format!("Removed the older build {}", d.display()));
        }
    }
}

/// Whether a running process's executable is inside dir, or whether that cannot be told (dir
/// does not resolve, a QEMU process's executable is unreadable).
fn in_use(proc_root: &Path, dir: &Path) -> bool {
    // /proc/<pid>/exe is canonical, so dir must be too (a Workspace may be behind a symlink).
    let Ok(dir) = fs::canonicalize(dir) else {
        return true;
    };
    let Ok(ents) = fs::read_dir(proc_root) else {
        return false;
    };
    for e in ents.flatten() {
        if !e
            .file_name()
            .to_string_lossy()
            .starts_with(|c: char| c.is_ascii_digit())
        {
            continue;
        }
        let p = e.path();
        match fs::read_link(p.join("exe")) {
            Ok(t) => {
                let t = fs::canonicalize(&t).unwrap_or(t);
                if t != dir && t.starts_with(&dir) {
                    return true;
                }
            }
            Err(_) => {
                if fs::read_to_string(p.join("comm")).is_ok_and(|c| c.starts_with("qemu-system")) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::unpack;
    use crate::testutil::*;

    #[test]
    fn ldd_missing_names_libraries_and_packages() {
        let out = "\tlinux-vdso.so.1 (0x00007ffd)\n\tlibslirp.so.0 => not found\n\tlibz.so.1 => /lib/x86_64-linux-gnu/libz.so.1 (0x1)\n\tlibfoo.so.9 => not found\n";
        let missing = ldd_missing(out);
        assert_eq!(missing, ["libslirp.so.0", "libfoo.so.9"]);
        let m = missing_libs_error(&missing).to_string();
        assert!(
            m.contains("libslirp.so.0 (libslirp0)") && m.contains("libfoo.so.9"),
            "{m}"
        );
        assert!(m.contains("sudo apt install libslirp0"), "{m}");
    }

    #[test]
    fn in_use_sees_processes_inside_a_build() -> TestResult {
        let t = TempDir::new()?;
        let real = t.path().join("real");
        let link = t.path().join("ws");
        fs::create_dir_all(real.join("b/bin"))?;
        std::os::unix::fs::symlink(&real, &link)?;
        let proc_root = t.path().join("proc");
        let pid = |n: &str, exe: Option<&Path>, comm: &str| -> std::io::Result<()> {
            let d = proc_root.join(n);
            fs::create_dir_all(&d)?;
            if let Some(exe) = exe {
                std::os::unix::fs::symlink(exe, d.join("exe"))?;
            }
            fs::write(d.join("comm"), format!("{comm}\n"))
        };
        pid("1", Some(Path::new("/usr/bin/bash")), "bash")?;
        pid("2", None, "kthreadd")?; // no exe: a kernel thread
        assert!(!in_use(&proc_root, &link.join("b")), "no process uses it");
        pid(
            "3",
            Some(&real.join("b/bin/qemu-system-x86_64")),
            "qemu-system-x86",
        )?;
        assert!(
            in_use(&proc_root, &link.join("b")),
            "the canonical exe is inside the build reached through a symlink"
        );
        fs::remove_dir_all(proc_root.join("3"))?;
        pid("4", None, "qemu-system-x86")?; // unreadable exe
        assert!(
            in_use(&proc_root, &link.join("b")),
            "a QEMU whose executable cannot be read keeps the build"
        );
        assert!(
            in_use(t.path(), &link.join("missing")),
            "a build that does not resolve is kept"
        );
        Ok(())
    }

    #[test]
    fn activate_switches_current_stages_other_releases_and_prunes() -> TestResult {
        let _procs = QEMU_PROCS.lock();
        let t = TempDir::new()?;
        let mut r = reims_in(&t, "ID=ubuntu\nVERSION_ID=\"24.04\"\n")?;
        r.ldd = "true".into();
        let pkg = fake_package()?;
        let qdir = t.path().join("qemu");
        let build = |name: &str| unpack(pkg.as_slice(), &qdir.join(name), crate::MAX_EXPANDED);
        let log = |_: &str| {};
        for b in [
            "old-ubuntu24.04",
            "busy-ubuntu24.04",
            "a-ubuntu26.04",
            "new-ubuntu24.04",
        ] {
            build(b)?;
        }
        write(
            &qdir.join("a-ubuntu26.04/bin/qemu-system-x86_64"),
            "#!/bin/sh\n",
        )?;

        // A running process inside a build keeps that build.
        let busy = qdir.join("busy-ubuntu24.04/bin/sleep");
        fs::copy("/bin/sleep", &busy)?;
        let mut child = process::spawn(Command::new(&busy).arg("30"))?;
        let res = (|| -> TestResult {
            activate(&r, &qdir, "a-ubuntu26.04", "26.04", &log)?;
            assert!(
                fs::symlink_metadata(qdir.join("current")).is_err(),
                "a build for another release stays staged"
            );
            activate(&r, &qdir, "new-ubuntu24.04", "24.04", &log)?;
            let mut left: Vec<_> = fs::read_dir(&qdir)?
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            left.sort();
            assert_eq!(
                left,
                [
                    "a-ubuntu26.04",
                    "busy-ubuntu24.04",
                    "current",
                    "new-ubuntu24.04"
                ]
            );
            Ok(())
        })();
        child.kill()?;
        child.wait()?;
        res?;

        // A build that fails on the host leaves current as it was.
        build("broken-ubuntu24.04")?;
        fs::remove_file(qdir.join("broken-ubuntu24.04/share/qemu/ReimsVgpu.rom"))?;
        let err = activate(&r, &qdir, "broken-ubuntu24.04", "24.04", &log)
            .err()
            .ok_or("activated")?;
        assert!(err.to_string().contains("staged"), "{err}");
        assert_eq!(
            fs::read_link(qdir.join("current"))?,
            Path::new("new-ubuntu24.04")
        );
        Ok(())
    }

    #[test]
    fn check_build_reports_missing_libraries() -> TestResult {
        let _procs = QEMU_PROCS.lock();
        let t = TempDir::new()?;
        let mut r = reims_in(&t, "")?;
        let ldd = t.path().join("ldd");
        write(&ldd, "#!/bin/sh\necho '\tlibvulkan.so.1 => not found'\n")?;
        r.ldd = ldd.to_string_lossy().into_owned();
        let dir = t.path().join("b");
        unpack(fake_package()?.as_slice(), &dir, crate::MAX_EXPANDED)?;
        let err = check_build(&r, &dir).err().ok_or("passed")?.to_string();
        assert!(err.contains("sudo apt install libvulkan1"), "{err}");
        Ok(())
    }

    #[test]
    fn probe_rejects_a_qemu_without_apple_gfx() -> TestResult {
        let _procs = QEMU_PROCS.lock();
        let t = TempDir::new()?;
        let q = t.path().join("q");
        write(&q, "#!/bin/sh\necho 'name \"VGA\"'\n")?;
        assert!(
            probe_apple_gfx(&q).is_err_and(|e| e.to_string().contains("not a Reims vGPU build"))
        );
        write(&q, "#!/bin/sh\necho broken >&2\nexit 3\n")?;
        assert!(probe_apple_gfx(&q).is_err_and(|e| e.to_string().contains("broken")));
        Ok(())
    }
}

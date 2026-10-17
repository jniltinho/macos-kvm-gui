//! Fake /proc, /sys and /etc trees for the tests (Go `fakeHost`, `fakeSysfs`).

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tempfile::TempDir;

use crate::host::{Check, CmdError, Host, open_rw};

pub type TestResult = Result<(), Box<dyn std::error::Error>>;

pub fn write(path: &Path, data: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, data)
}

pub fn cmd_err(code: i32, output: &str) -> CmdError {
    CmdError {
        code: Some(code),
        output: output.into(),
    }
}

/// Whether `p`, just made mode 0, still opens: we run as root and permission checks are moot.
pub fn is_root_for(p: &Path) -> bool {
    open_rw(p).is_ok()
}

pub fn by_name(cs: Vec<Check>) -> HashMap<String, Check> {
    cs.into_iter().map(|c| (c.name.clone(), c)).collect()
}

/// A host ready for KVM and Reims vGPU (NVIDIA Vulkan ICD, large map count) in a temp folder.
pub fn fake_host(cpu_flags: &str, msrs: &str, procs: &[&str]) -> io::Result<(TempDir, Host)> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let reims: PathBuf = root.join("reims-qemu");
    write(&reims, "")?;
    let h = Host {
        proc_root: root.join("proc"),
        sys_root: root.join("sys"),
        etc_root: root.join("etc"),
        dev_kvm: root.join("kvm"),
        dev_vfio: root.join("dev-vfio"),
        share_root: root.join("share"),
        dev_udmabuf: root.join("udmabuf"),
        look_path: Arc::new(|s| Some(PathBuf::from("/usr/bin").join(s))),
        output: Arc::new(|_, _| {
            Ok(b"QEMU emulator version 8.2.2 (Debian 1:8.2.2+ds-0ubuntu1)\nname \"apple-gfx-pci\", bus PCI".to_vec())
        }),
        run: Arc::new(|_, _| Ok(())),
        reims_qemu: Arc::new(move || Ok(reims.clone())),
        extra_kexts: Arc::new(|| Ok(0)),
        settings_file: root
            .join("config/settings.yaml")
            .to_string_lossy()
            .into_owned(),
    };
    write(&h.share_root.join("vulkan/icd.d/nvidia_icd.json"), "{}")?;
    write(&h.proc_root.join("sys/vm/max_map_count"), "2097152\n")?;
    write(
        &h.proc_root.join("cpuinfo"),
        &format!("processor : 0\nflags\t\t: fpu {cpu_flags}\n"),
    )?;
    if !msrs.is_empty() {
        write(&h.msrs_path(), &format!("{msrs}\n"))?;
    }
    for (i, p) in procs.iter().enumerate() {
        write(
            &h.proc_root.join((i + 1).to_string()).join("comm"),
            &format!("{p}\n"),
        )?;
    }
    write(&h.dev_kvm, "")?;
    Ok((dir, h))
}

//! Port of the Go app's cmd/cmd_test.go.

use super::*;
use crate::testutil::Env;
use anyhow::ensure;

/// Runs a command; returns stdout and stderr together, and the error as `{:#}`.
fn run(env: &Env, args: &[&str]) -> (String, Result<(), String>) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let res = execute(args, &env.ctx, &mut out, &mut err).map_err(|e| format!("{e:#}"));
    out.extend(err);
    (String::from_utf8_lossy(&out).into_owned(), res)
}

fn ok(env: &Env, args: &[&str]) -> Result<String> {
    match run(env, args) {
        (out, Ok(())) => Ok(out),
        (out, Err(e)) => bail!("{args:?}: {e}\n{out}"),
    }
}

fn fails(env: &Env, args: &[&str], want: &str) -> Result<String> {
    let (out, res) = run(env, args);
    match res {
        Err(e) if e.contains(want) => Ok(out),
        other => bail!("{args:?} must fail with {want:?}: {other:?}\n{out}"),
    }
}

#[test]
fn config_commands() -> Result<()> {
    let env = Env::new()?;
    let cfg = env.path("config.json").display().to_string();
    let c = |rest: &[&'static str]| -> Vec<&str> {
        ["--config", cfg.as_str()]
            .into_iter()
            .chain(rest.iter().copied())
            .collect()
    };
    ok(
        &env,
        &c(&[
            "config",
            "set",
            "version=sonoma",
            "ram=16384",
            "cores=8",
            "vcpus=16",
            "display=window",
        ]),
    )?;
    let out = ok(&env, &c(&["config", "show"]))?;
    ensure!(
        out.contains(r#""ram_mib": 16384"#) && out.contains(r#""display": "window""#),
        "show:\n{out}"
    );
    fails(&env, &c(&["config", "set", "vcpus=6"]), "vcpus")?; // 6 vCPUs on 8 cores
    fails(&env, &c(&["config", "set", "nope=1"]), "known key")?;
    let out = ok(&env, &c(&["config", "show"]))?;
    ensure!(
        out.contains(r#""graphics": "vmware-svga""#),
        "show must print the default graphics:\n{out}"
    );
    fails(
        &env,
        &c(&["config", "set", "graphics=virtio"]),
        "vmware-svga or reims-vgpu",
    )?;
    fails(
        &env,
        &c(&["config", "set", "graphics=reims-vgpu", "gpu=0000:03:00.0"]),
        "choose one",
    )?;
    ok(&env, &c(&["config", "set", "graphics=reims-vgpu"]))?;
    let out = ok(&env, &c(&["config", "show"]))?;
    ensure!(
        out.contains(r#""graphics": "reims-vgpu""#) && out.contains(r#""resolution": "1920x1080""#),
        "{out}"
    );
    fails(
        &env,
        &c(&["config", "set", "resolution=1600x900"]),
        "1920x1080, 1920x1200",
    )?;
    let out = ok(&env, &c(&["config", "set", "resolution=2560x1440"]))?;
    ensure!(
        out.contains("between 2048 and 3840 px"),
        "a wide resolution before Tahoe warns: {out}"
    );
    ensure!(ok(&env, &c(&["config", "show"]))?.contains(r#""resolution": "2560x1440""#));
    fails(
        &env,
        &c(&["config", "set", "root-patch=maybe"]),
        "is not true or false",
    )?;
    fails(&env, &c(&["media", "remove"]), "--yes")?;
    let out = ok(&env, &["version"])?;
    ensure!(
        out.contains("reims-gui test") && out.contains("OpenCore"),
        "version: {out}"
    );
    Ok(())
}

#[test]
fn vm_commands() -> Result<()> {
    let env = Env::new()?;
    let ws = env.path("tahoe").display().to_string();
    let out = ok(
        &env,
        &[
            "vm",
            "create",
            "Tahoe lab",
            "--version",
            "tahoe",
            "--folder",
            &ws,
        ],
    )?;
    ensure!(
        out.contains("created and selected Tahoe lab"),
        "create: {out}"
    );
    let out = ok(&env, &["vm", "list"])?;
    ensure!(
        out.contains("*  Tahoe lab") && out.contains("stopped"),
        "list must mark the selected VM:\n{out}"
    );
    ensure!(ok(&env, &["--vm", "Tahoe lab", "config", "show"])?.contains(r#""version": "tahoe""#));
    fails(
        &env,
        &["--vm", "nope", "config", "show"],
        "no VM named \"nope\"",
    )?;
    ok(
        &env,
        &[
            "config",
            "set",
            "nic2.enabled=true",
            "nic2.mode=bridged",
            "nic2.bridge=br0",
            "nic2.model=vmxnet3",
            "nic2.mac=random",
        ],
    )?;
    let out = ok(&env, &["config", "show"])?;
    ensure!(
        out.contains(r#""bridge": "br0""#) && out.matches(r#""mac""#).count() == 2,
        "adapter 2 must be saved next to the VM's own adapter:\n{out}"
    );
    fails(&env, &["config", "set", "nic2.mac=01:00:5e:00:00:01"], "")?; // multicast
    fails(&env, &["config", "set", "nic5.enabled=true"], "known key")?;
    let out = ok(&env, &["vm", "status"])?;
    ensure!(
        out.contains("name:      Tahoe lab") && out.contains("status:    stopped"),
        "{out}"
    );
    let root = env.path("vms").display().to_string();
    ok(&env, &["workspace", "set", &root])?;
    ensure!(ok(&env, &["workspace", "show"])?.trim() == root);
    let out = ok(&env, &["vm", "move", "Tahoe lab"])?;
    ensure!(out.contains(&format!("{root}/tahoe-lab")), "move: {out}");
    let out = ok(&env, &["vm", "export", "Tahoe lab"])?;
    ensure!(out.contains(r#""name": "Tahoe lab""#), "export: {out}");
    fails(
        &env,
        &["vm", "export", "Tahoe lab", "--full"],
        "--full needs a destination file",
    )?;
    fails(&env, &["vm", "delete", "Tahoe lab"], "--yes")?;
    let out = ok(&env, &["vm", "delete", "Tahoe lab", "--purge", "--yes"])?;
    ensure!(out.contains("deleted"), "delete: {out}");
    Ok(())
}

#[test]
fn settings_and_kexts_before_start() -> Result<()> {
    let env = Env::new()?;
    fails(&env, &["settings", "reset"], "--yes")?;
    ensure!(ok(&env, &["settings", "reset", "--yes"])?.contains("settings.yaml"));
    let ws = env.path("lab").display().to_string();
    ok(
        &env,
        &[
            "vm",
            "create",
            "Lab",
            "--version",
            "sequoia",
            "--folder",
            &ws,
        ],
    )?;
    // An entry that cannot be downloaded stops vm start before anything else: no VM runs.
    std::fs::write(
        env.ctx.app.settings_file(),
        "kexts:\n  - url: https://127.0.0.1:1/x.zip\n",
    )?;
    let out = ok(&env, &["settings", "show"])?;
    ensure!(
        out.contains("kext: https://127.0.0.1:1/x.zip") && out.contains("kexts downloaded: false"),
        "show: {out}"
    );
    let out = fails(&env, &["vm", "start"], "127.0.0.1:1")?;
    ensure!(
        out.contains("downloading the kexts"),
        "vm start downloads first: {out}"
    );
    Ok(())
}

#[test]
fn reims_commands() -> Result<()> {
    let env = Env::new()?;
    fails(
        &env,
        &["reims", "build", "--release", "22.04"],
        "24.04 and 26.04",
    )?;
    fails(&env, &["reims", "status"], "not installed")?;
    let qemu = env.root().join(".reims/qemu");
    let bin = qemu.join("20261016-212623-c099dd5a-ubuntu24.04/bin");
    std::fs::create_dir_all(&bin)?;
    std::fs::write(
        bin.join("qemu-system-x86_64"),
        "#!/bin/sh\necho 'name \"apple-gfx-pci\", bus PCI'\n",
    )?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        bin.join("qemu-system-x86_64"),
        std::fs::Permissions::from_mode(0o755),
    )?;
    std::os::unix::fs::symlink("20261016-212623-c099dd5a-ubuntu24.04", qemu.join("current"))?;
    let out = ok(&env, &["reims", "status"])?;
    ensure!(
        out.contains("commit: c099dd5a") && out.contains("apple-gfx-pci: yes"),
        "status:\n{out}"
    );
    Ok(())
}

#[test]
fn installers_and_gpus_list_with_headers() -> Result<()> {
    let env = Env::new()?;
    ensure!(ok(&env, &["installer", "list"])?.starts_with("VERSION  BUILD  SIZE  PATH"));
    ensure!(ok(&env, &["gpu", "list"])?.starts_with("ADDRESS"));
    fails(&env, &["installer", "remove", "../x"], "invalid build")?;
    Ok(())
}

#[test]
fn unknown_commands_and_flags_are_errors() -> Result<()> {
    let env = Env::new()?;
    ensure!(run(&env, &["nope"]).1.is_err());
    ensure!(run(&env, &["vm", "start", "--nope"]).1.is_err());
    Ok(())
}

#[test]
fn table_pads_every_cell_but_the_last() {
    let rows = vec![
        vec!["".into(), "NAME".into(), "X".into()],
        vec!["*".into(), "Lab".into(), "y".into()],
    ];
    assert_eq!(table(&rows), "   NAME  X\n*  Lab   y\n");
}

/// Design D1 through the CLI: no flag (automatic), an unused explicit port, another VM's port
/// (notice), 80 (refused, no VM); then an import, `config set` and `vm status` warnings.
#[test]
fn vm_create_and_import_pick_free_ssh_ports() -> Result<()> {
    let env = Env::new()?;
    let out = ok(&env, &["vm", "create", "Nilton"])?;
    ensure!(
        out.contains("SSH port 2222") && !out.contains("belongs"),
        "{out}"
    );
    let out = ok(&env, &["vm", "create", "B", "--ssh-port", "2300"])?;
    ensure!(
        out.contains("SSH port 2300") && !out.contains("belongs"),
        "{out}"
    );
    let out = ok(&env, &["vm", "create", "C", "--ssh-port", "2222"])?;
    ensure!(
        out.contains("SSH port 2222 belongs to Nilton; using 2223\n")
            && out.contains("SSH port 2223"),
        "{out}"
    );
    fails(
        &env,
        &["vm", "create", "Low", "--ssh-port", "80"],
        "use a port between 1024 and 65535",
    )?;
    ensure!(
        !ok(&env, &["vm", "list"])?.contains("Low"),
        "no VM is added"
    );

    let file = env.path("nilton.json").display().to_string();
    ok(&env, &["vm", "export", "Nilton", &file])?;
    let out = ok(&env, &["vm", "import", &file])?;
    ensure!(
        out.contains("SSH port 2222 belongs to Nilton; using 2224")
            && out.contains("imported Nilton (imported)"),
        "{out}"
    );

    // A shared port is saved with a warning on stderr.
    let (mut o, mut e) = (Vec::new(), Vec::new());
    execute(
        &["--vm", "B", "config", "set", "ssh-port=2222"],
        &env.ctx,
        &mut o,
        &mut e,
    )?;
    let e = String::from_utf8_lossy(&e);
    ensure!(
        e == "warning: Also used by Nilton: they cannot run at the same time\n",
        "{e:?}"
    );
    let out = ok(&env, &["vm", "status", "Nilton"])?;
    ensure!(
        out.contains("warning:   Also used by B: they cannot run at the same time"),
        "{out}"
    );
    ensure!(!ok(&env, &["vm", "status", "C"])?.contains("warning"));
    Ok(())
}

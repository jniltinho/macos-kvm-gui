//! `reims-gui --self-test` under Xvfb: the MCP self-check, then the GUI on the real backend
//! draws frames and exits 0 (lavapipe in the build image). Skipped without xvfb-run.

use std::process::Command;

#[test]
fn self_test_draws_frames_under_xvfb() -> Result<(), Box<dyn std::error::Error>> {
    if Command::new("xvfb-run").arg("--help").output().is_err() {
        eprintln!("skipped: xvfb-run is not installed");
        return Ok(());
    }
    let home = tempfile::tempdir()?;
    let out = Command::new("xvfb-run")
        .args(["-a", env!("CARGO_BIN_EXE_reims-gui"), "--self-test"])
        // An empty HOME shows the self-test writes no user data.
        .env("HOME", home.path())
        .env_remove("XDG_CONFIG_HOME")
        .output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("MCP tools"), "{stdout}");
    assert!(stdout.contains("the window drew"), "{stdout}");
    assert!(
        !home.path().join(".config/macos-kvm-gui").exists(),
        "the user's config folder was created"
    );
    Ok(())
}

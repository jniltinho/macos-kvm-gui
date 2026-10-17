//! The offline install: with the cached full installer attached as a disc next to the recovery,
//! the app opens the recovery's Terminal and types the disc's install line (Go GUI `typeLine`,
//! MCP `install_offline`).

use std::time::Duration;

use rg_core::Config;
use rg_macos::{DISC_COMMAND, DISC_TYPED};
use rg_qemu::blocking::send_chords;
use rg_qemu::keys::{guest_keys, recovery_terminal};

use crate::{Ops, Result, msg};

/// What the user types in the recovery's Terminal (`-u`: unattended, erases the empty disk and
/// accepts Apple's license).
pub fn install_line(unattended: bool) -> String {
    if unattended {
        format!("{DISC_COMMAND} -u")
    } else {
        DISC_COMMAND.into()
    }
}

impl Ops {
    /// Opens Utilities › Terminal in the running VM's recovery and types the install line from
    /// the attached installer disc. The unattended install needs the user's consent to erase
    /// the disk and accept the license. Blocking (a few seconds of typing); not inside a tokio
    /// runtime.
    pub fn install_offline(
        &self,
        c: &Config,
        unattended: bool,
        accept_license: bool,
        erase_disk: bool,
    ) -> Result<()> {
        if unattended && !accept_license {
            return Err(msg(
                "the unattended install accepts Apple's macOS license for the user: ask them first",
            ));
        }
        if unattended && !erase_disk {
            return Err(msg(
                "the unattended install erases the VM's empty disk: ask the user first",
            ));
        }
        if !rg_core::keyboard::guest_typing_ok() {
            return Err(msg(format!(
                "this keyboard layout moves the letters of the line (Dvorak, Russian): type it in the recovery's Terminal yourself: {DISC_COMMAND}"
            )));
        }
        let r = self
            .find_running(c)
            .ok_or_else(|| msg(format!("{} is not running", c.name)))?;
        if !r.disc {
            return Err(msg(format!(
                "{} has no offline disc: download the installer, then start the VM with its install media",
                c.name
            )));
        }
        let vnc = r
            .vnc_socket
            .ok_or_else(|| msg("the VM has no VNC socket to type into"))?;
        let mut line = DISC_TYPED.to_string();
        if unattended {
            line += " -u";
        }
        let chords = guest_keys(&line)?;
        send_chords(&vnc, &recovery_terminal(), Duration::from_millis(120))?; // the menu opens
        std::thread::sleep(Duration::from_secs(2)); // the Terminal window opens
        send_chords(&vnc, &chords, Duration::ZERO)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[test]
    fn install_line_adds_unattended() {
        assert_eq!(install_line(false), "sh /Volumes/OFFLINE/INSTALL.SH");
        assert_eq!(install_line(true), "sh /Volumes/OFFLINE/INSTALL.SH -u");
    }

    #[test]
    fn install_offline_needs_consent_a_running_vm_and_its_disc() -> TestResult {
        let t = tempfile::tempdir()?;
        let o = ops(&t)?;
        let c = config(&t, "inst");
        let text = |r: Result<()>| r.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(text(o.install_offline(&c, true, false, true)).contains("license"));
        assert!(text(o.install_offline(&c, true, true, false)).contains("erases"));
        if !rg_core::keyboard::guest_typing_ok() {
            return Ok(()); // the host's layout stops it earlier
        }
        assert!(text(o.install_offline(&c, false, false, false)).contains("not running"));
        std::fs::create_dir_all(c.workspace_dir()?)?;
        fake_qemu(&o, &c, 99)?;
        assert!(text(o.install_offline(&c, false, false, false)).contains("no offline disc"));
        Ok(())
    }
}

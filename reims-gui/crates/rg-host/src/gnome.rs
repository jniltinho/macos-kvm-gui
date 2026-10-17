//! GNOME on Wayland: whether GNOME's shortcuts (Super, Alt+Tab, Alt+F2) reach the guest while it
//! has the keyboard.
//!
//! The guest's window is an X11 one (XWayland), and Mutter ignores XWayland keyboard grabs unless
//! `org.gnome.mutter.wayland xwayland-allow-grabs` is on and the grabbing window's toplevel is
//! in `xwayland-grab-access-rules` (its WM_CLASS). Measured on GNOME Shell 46 and 50: with both
//! set, even at runtime, Mutter logs "has a grab granted" for the app's window and keeps its
//! shortcuts out of the guest; without them, Super opens the overview.

use crate::host::{Check, Host, HostError, Status, failed};

/// The app's WM_CLASS (its winit app id), the rule Mutter matches.
pub const APP_CLASS: &str = "io.github.jniltinho.reims-gui";

const SCHEMA: &str = "org.gnome.mutter.wayland";
const ALLOW: &str = "xwayland-allow-grabs";
const RULES: &str = "xwayland-grab-access-rules";
const CHECK: &str = "GNOME shortcuts in the guest";

impl Host {
    /// `None` outside a GNOME Wayland session.
    pub fn check_gnome_capture(&self) -> Option<Check> {
        if !self.gnome_wayland {
            return None;
        }
        Some(match self.gnome_capture_state() {
            Err(e) => Check::new(CHECK, Status::Unknown, format!("gsettings: {e}"), ""),
            Ok((true, rules)) if rules.iter().any(|r| r == APP_CLASS) => Check::new(
                CHECK,
                Status::Ok,
                "GNOME lets the guest keep the keyboard (Super, Alt+Tab go to macOS while captured)",
                "",
            ),
            Ok(_) => Check::new(
                CHECK,
                Status::Warn,
                "GNOME keeps its shortcuts (Super, Alt+Tab, Alt+F2) even while the guest has the keyboard",
                format!(
                    "press Set up host, or: gsettings set {SCHEMA} {ALLOW} true; gsettings set {SCHEMA} {RULES} \"['{APP_CLASS}']\""
                ),
            ),
        })
    }

    /// Turns the setting on for this user, keeping the other rules. Whether it changed anything.
    pub fn enable_gnome_capture(&self) -> Result<bool, HostError> {
        if !self.gnome_wayland {
            return Ok(false);
        }
        let (allow, mut rules) = self.gnome_capture_state().map_err(failed)?;
        let has = rules.iter().any(|r| r == APP_CLASS);
        if allow && has {
            return Ok(false);
        }
        if !has {
            rules.push(APP_CLASS.into());
        }
        let set = |key: &str, value: &str| {
            (self.run)("gsettings", &["set", SCHEMA, key, value])
                .map_err(|e| failed(format!("gsettings set {SCHEMA} {key}: {e}")))
        };
        set(RULES, &format_list(&rules))?;
        set(ALLOW, "true")?;
        Ok(true)
    }

    fn gnome_capture_state(&self) -> Result<(bool, Vec<String>), String> {
        let get = |key: &str| {
            (self.output)("gsettings", &["get", SCHEMA, key])
                .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                .map_err(|e| e.to_string())
        };
        Ok((get(ALLOW)? == "true", parse_list(&get(RULES)?)))
    }
}

/// Whether this process runs in a GNOME session on Wayland.
pub fn gnome_wayland_session() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    desktop.split(':').any(|d| d.eq_ignore_ascii_case("GNOME"))
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// A GVariant string array as gsettings prints it: `['a', 'b']`, `@as []`.
fn parse_list(s: &str) -> Vec<String> {
    let s = s.trim().trim_start_matches("@as").trim();
    s.trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|r| r.trim().trim_matches(|c| c == '\'' || c == '"').to_owned())
        .filter(|r| !r.is_empty())
        .collect()
}

/// The GVariant text for `rules` (names hold no quotes or commas: window classes).
fn format_list(rules: &[String]) -> String {
    let quoted: Vec<_> = rules.iter().map(|r| format!("'{r}'")).collect();
    format!("[{}]", quoted.join(", "))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::testutil::fake_host;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn lists_round_trip() {
        assert_eq!(parse_list("@as []"), Vec::<String>::new());
        assert_eq!(parse_list("['a', 'b.c']"), ["a", "b.c"]);
        assert_eq!(format_list(&["a".into(), "b.c".into()]), "['a', 'b.c']");
    }

    /// gsettings answers from `state` and `set` writes into it.
    fn with_gsettings(h: &mut Host, allow: &str, rules: &str) -> Arc<Mutex<Vec<(String, String)>>> {
        let state = Arc::new(Mutex::new(vec![
            (ALLOW.to_string(), allow.to_string()),
            (RULES.to_string(), rules.to_string()),
        ]));
        let read = state.clone();
        h.output = Arc::new(move |_, a| {
            let s = read
                .lock()
                .map_err(|_| crate::testutil::cmd_err(1, "lock"))?;
            Ok(s.iter()
                .find(|(k, _)| k == a[2])
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
                .into_bytes())
        });
        let write = state.clone();
        h.run = Arc::new(move |_, a| {
            let mut s = write
                .lock()
                .map_err(|_| crate::testutil::cmd_err(1, "lock"))?;
            if let Some(e) = s.iter_mut().find(|(k, _)| k == a[2]) {
                e.1 = a[3].to_string();
            }
            Ok(())
        });
        h.gnome_wayland = true;
        state
    }

    #[test]
    fn only_in_gnome_on_wayland() -> TestResult {
        let (_d, h) = fake_host("svm", "Y", &[])?;
        assert!(h.check_gnome_capture().is_none());
        assert!(!h.enable_gnome_capture()?);
        Ok(())
    }

    #[test]
    fn setup_turns_it_on_and_keeps_other_rules() -> TestResult {
        let (_d, mut h) = fake_host("svm", "Y", &[])?;
        let state = with_gsettings(&mut h, "false", "['virt-manager']");
        assert_eq!(
            h.check_gnome_capture().map(|c| c.status),
            Some(Status::Warn)
        );
        assert!(h.enable_gnome_capture()?);
        assert_eq!(h.check_gnome_capture().map(|c| c.status), Some(Status::Ok));
        let s = state.lock().map_err(|_| "lock")?;
        assert_eq!(s[1].1, format!("['virt-manager', '{APP_CLASS}']"));
        drop(s);
        assert!(!h.enable_gnome_capture()?, "nothing to do the second time");
        Ok(())
    }
}

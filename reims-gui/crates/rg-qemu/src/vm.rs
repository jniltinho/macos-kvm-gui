//! Running one QEMU: the private run directory, the process, its output, QMP control and the
//! VM's state (Go `vm.go` `VM`).
//!
//! The checks and preparation that need other crates (config validation, the Workspace lock,
//! boot files, bridges, the SSH port, GPU and Reims host checks, the installer disc) are the
//! caller's: it passes their results in a [`StartSpec`], and what must happen when QEMU exits
//! (unlock, release the disc, clear the root-patch marker) as [`StartSpec::on_exit`].
//!
//! QEMU must not outlive the app, even when the app is killed. Go sets `Pdeathsig` on the
//! child; here `unsafe` is forbidden (`pre_exec` is unsafe), so QEMU is started through
//! util-linux's `setpriv --pdeathsig TERM`, which sets it and execs QEMU (same pid, same
//! `/proc/<pid>/comm`). The signal fires when the *thread* that forked dies, so the spawn and
//! the wait run on one dedicated supervisor thread that ends only after QEMU does. (rustix's
//! `set_parent_process_death_signal` sets it on the calling process, so it too would need
//! `pre_exec`.) The wait itself does not poll: tokio's child reaper waits on a pidfd. QEMU
//! gets its own process group (Ctrl+C in a terminal must not reach it); std's pipes and
//! sockets are O_CLOEXEC, so no other descriptor leaks into QEMU.

use crate::args::{ArgsEnv, build_args, vnc_socket};
use crate::qmp::{Qmp, QmpEvent};
use crate::running::{RUN_DIR_PREFIX, find_running};
use crate::{Error, Result};
use parking_lot::Mutex;
use rg_core::Config;
use rg_core::workspace::DISK_FILE;
use std::io::BufRead;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use tokio::sync::Notify;

/// Where util-linux installs setpriv.
const SETPRIV: [&str; 2] = ["/usr/bin/setpriv", "/bin/setpriv"];
/// QEMU output lines kept for [`explain_exit`].
const TAIL: usize = 20;

/// A VM's state as the app shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VmState {
    #[default]
    Stopped,
    Running,
    /// Paused by QEMU (an I/O error, or STOP).
    Paused,
}

impl std::fmt::Display for VmState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Stopped => "Stopped",
            Self::Running => "Running",
            Self::Paused => "Paused (error)",
        })
    }
}

/// What a [`Vm`] reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VmEvent {
    /// The state and a message (an error, a warning, or empty).
    State(VmState, String),
    /// A line for the Log tab: the command line, QEMU's output, warnings.
    Log(String),
}

/// Type of [`StartSpec::on_exit`]: called with how long QEMU ran.
pub type OnExit = Box<dyn FnOnce(Duration) + Send>;

/// One start, prepared by the caller.
#[derive(Default)]
pub struct StartSpec {
    pub config: Config,
    /// `qemu-system-x86_64`, or the Reims vGPU build for a Reims VM.
    pub qemu: PathBuf,
    /// Attach the recovery image.
    pub with_media: bool,
    /// The cached installer disc to attach with the recovery.
    pub installer_disc: Option<PathBuf>,
    /// A passed-through GPU's PCI functions (from the GPU checks).
    pub gpu_funcs: Vec<String>,
    /// Reims vGPU: an X11 display is usable and the app will embed the library's window, so
    /// QEMU gets `SDL_VIDEO_DRIVER=x11`. Off leaves SDL's own choice (native Wayland).
    pub x11_embed: bool,
    /// VMware SVGA: QEMU's own SDL window for the app to embed (see [`ArgsEnv::sdl_window`]),
    /// with `SDL_VIDEO_DRIVER=x11`.
    pub sdl_window: bool,
    /// Runs once QEMU has exited (or failed to start), after the run directory is gone.
    pub on_exit: Option<OnExit>,
}

/// A started QEMU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Started {
    /// QEMU's pid (the GUI finds its window by `_NET_WM_PID`).
    pub pid: u32,
    pub qmp_socket: PathBuf,
    /// The VNC socket, when QEMU serves one.
    pub vnc_socket: Option<PathBuf>,
}

#[derive(Default)]
struct St {
    state: VmState,
    pid: Option<u32>,
    qmp: Option<Arc<Qmp>>,
    vnc_sock: Option<PathBuf>,
    disc: bool,
    io_error: String,
    killed: bool,
    kill: Option<Arc<Notify>>,
}

struct Inner {
    rt: Handle,
    on_event: Box<dyn Fn(VmEvent) + Send + Sync>,
    st: Mutex<St>,
}

/// Controls a QEMU started by this app instance. Cheap to clone.
#[derive(Clone)]
pub struct Vm(Arc<Inner>);

impl Vm {
    /// A stopped VM; `on_event` hears its state changes and log lines from any thread.
    pub fn new(rt: Handle, on_event: impl Fn(VmEvent) + Send + Sync + 'static) -> Self {
        Self(Arc::new(Inner {
            rt,
            on_event: Box::new(on_event),
            st: Mutex::new(St::default()),
        }))
    }

    pub fn state(&self) -> VmState {
        self.0.st.lock().state
    }

    /// QEMU's pid while it runs.
    pub fn pid(&self) -> Option<u32> {
        self.0.st.lock().pid
    }

    /// The VNC socket while running with one.
    pub fn vnc_socket(&self) -> Option<PathBuf> {
        self.0.st.lock().vnc_sock.clone()
    }

    /// Whether this run has the offline installer disc attached.
    pub fn disc_attached(&self) -> bool {
        self.0.st.lock().disc
    }

    /// The QMP connection, once QEMU accepted it.
    pub fn qmp(&self) -> Option<Arc<Qmp>> {
        self.0.st.lock().qmp.clone()
    }

    fn log(&self, s: impl Into<String>) {
        (self.0.on_event)(VmEvent::Log(s.into()));
    }

    fn set_state(&self, s: VmState, msg: impl Into<String>) {
        self.0.st.lock().state = s;
        (self.0.on_event)(VmEvent::State(s, msg.into()));
    }

    /// Starts QEMU. Returns once the process runs; its exit is reported as a `Stopped` event.
    pub fn start(&self, mut spec: StartSpec) -> Result<Started> {
        if self.state() != VmState::Stopped {
            return Err(Error::msg("the VM is already running"));
        }
        let c = &spec.config;
        let dir = c.workspace_dir().map_err(|e| Error::msg(e.to_string()))?;
        if !dir.is_dir() {
            return Err(Error::msg(format!(
                "VM folder {} does not exist: create it in the Install tab",
                dir.display()
            )));
        }
        if !dir.join(DISK_FILE).exists() {
            return Err(Error::msg(format!(
                "{DISK_FILE} does not exist: create the disk in the Install tab"
            )));
        }
        if let Some(r) = find_running(c) {
            let who = if r.by_app {
                "this app"
            } else {
                "another program (for example OSX-KVM's OpenCore-Boot.sh)"
            };
            return Err(Error::msg(format!(
                "this VM's disk is already in use by QEMU (pid {}), started by {who}: shut that VM down first",
                r.pid
            )));
        }
        let run_dir = private_run_dir(&runtime_base())?;
        let sock = run_dir.join("qmp.sock");
        let mut env = ArgsEnv::host(&dir, &sock);
        env.with_media = spec.with_media;
        env.installer_disc = spec.installer_disc.clone();
        env.gpu_funcs = std::mem::take(&mut spec.gpu_funcs);
        env.sdl_window = spec.sdl_window;
        let args = build_args(c, &env);
        let serves_vnc = spec.sdl_window || c.in_app() || c.reims();
        self.log(format!("{} {}", spec.qemu.display(), args.join(" ")));

        let started = Started {
            pid: 0,
            qmp_socket: sock.clone(),
            vnc_socket: serves_vnc.then(|| vnc_socket(&sock)),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let vm = self.clone();
        let spawn = std::thread::Builder::new()
            .name("qemu-supervisor".into())
            .spawn(move || {
                vm.supervise(spec, dir, args, run_dir, started, &tx);
            });
        if let Err(e) = spawn {
            return Err(Error::msg(format!("could not start QEMU: {e}")));
        }
        rx.recv().map_err(|_| Error::msg("could not start QEMU"))?
    }

    /// The supervisor thread: spawns QEMU, reports the start, waits for the exit and cleans up.
    fn supervise(
        &self,
        mut spec: StartSpec,
        dir: PathBuf,
        args: Vec<String>,
        run_dir: PathBuf,
        mut started: Started,
        tx: &std::sync::mpsc::Sender<Result<Started>>,
    ) {
        let on_exit = spec.on_exit.take();
        let finish = |on_exit: Option<OnExit>, ran: Duration| {
            let _ = std::fs::remove_dir_all(&run_dir);
            if let Some(f) = on_exit {
                f(ran);
            }
        };
        let _rt = self.0.rt.enter(); // tokio's process driver
        let child = match self.spawn(&spec, &dir, &args) {
            Ok(c) => c,
            Err(e) => {
                finish(on_exit, Duration::ZERO);
                let _ = tx.send(Err(Error::msg(format!("could not start QEMU: {e}"))));
                return;
            }
        };
        let (mut child, out) = child;
        let kill = Arc::new(Notify::new());
        started.pid = child.id().unwrap_or_default();
        {
            let mut st = self.0.st.lock();
            st.pid = Some(started.pid);
            st.vnc_sock.clone_from(&started.vnc_socket);
            st.disc = spec.with_media && spec.installer_disc.is_some();
            st.io_error.clear();
            st.killed = false;
            st.kill = Some(Arc::clone(&kill));
        }
        self.set_state(VmState::Running, ""); // before any Stopped from this thread
        let _ = tx.send(Ok(started.clone()));

        let vm = self.clone();
        let logger = std::thread::spawn(move || {
            let mut tail = std::collections::VecDeque::with_capacity(TAIL + 1);
            for line in std::io::BufReader::new(out)
                .lines()
                .map_while(std::io::Result::ok)
            {
                vm.log(line.clone());
                tail.push_back(line);
                if tail.len() > TAIL {
                    tail.pop_front();
                }
            }
            Vec::from(tail)
        });
        self.0
            .rt
            .spawn(self.clone().connect_qmp(started.qmp_socket.clone()));

        let t0 = Instant::now();
        let status = self.0.rt.block_on(async {
            tokio::select! {
                s = child.wait() => s,
                () = kill.notified() => {
                    let _ = child.start_kill();
                    child.wait().await
                }
            }
        });
        let ran = t0.elapsed();
        let tail = logger.join().unwrap_or_default();
        let killed = {
            let mut st = self.0.st.lock();
            let killed = st.killed;
            *st = St {
                state: st.state,
                ..St::default()
            };
            killed
        };
        finish(on_exit, ran);
        let msg = if killed {
            "Stopped by the user (forced).".to_string()
        } else {
            match status {
                Ok(s) => explain_exit(exit_error(s).as_deref(), &tail),
                Err(e) => explain_exit(Some(&e.to_string()), &tail),
            }
        };
        self.set_state(VmState::Stopped, msg);
    }

    fn spawn(
        &self,
        spec: &StartSpec,
        dir: &Path,
        args: &[String],
    ) -> std::io::Result<(tokio::process::Child, std::io::PipeReader)> {
        let setpriv = SETPRIV.iter().find(|p| Path::new(p).is_file());
        let mut cmd = match setpriv {
            Some(sp) => {
                let mut c = tokio::process::Command::new(sp);
                c.args(["--pdeathsig", "TERM", "--"]).arg(&spec.qemu);
                c
            }
            None => {
                self.log("warning: setpriv (util-linux) not found: QEMU keeps running if the app is killed");
                tokio::process::Command::new(&spec.qemu)
            }
        };
        cmd.args(args).current_dir(dir).stdin(Stdio::null());
        // Ctrl+C in a terminal must not reach QEMU.
        cmd.process_group(0);
        let reims = spec.config.reims();
        if reims {
            // The library opens its window only without REIMS_VGPU_WINDOW.
            cmd.env_remove("REIMS_VGPU_WINDOW");
        }
        // An SDL window the app embeds must be an X11 one (design D3).
        if (reims && spec.x11_embed) || (!reims && spec.sdl_window) {
            cmd.env("SDL_VIDEO_DRIVER", "x11");
        }
        let (r, w) = std::io::pipe()?;
        cmd.stdout(w.try_clone()?).stderr(w);
        let child = cmd.spawn()?;
        drop(cmd); // closes our copies of the pipe's write end
        Ok((child, r))
    }

    async fn connect_qmp(self, sock: PathBuf) {
        let (q, mut events) = match Qmp::dial(&sock).await {
            Ok(q) => q,
            Err(e) => {
                self.log(format!("warning: {e}"));
                return;
            }
        };
        {
            let mut st = self.0.st.lock();
            if st.pid.is_none() {
                return; // QEMU already exited
            }
            st.qmp = Some(Arc::new(q));
        }
        while let Some(e) = events.recv().await {
            self.on_qmp_event(&e);
        }
        let quiet = {
            let st = self.0.st.lock();
            st.killed || st.pid.is_none()
        };
        if !quiet {
            self.log("warning: QMP connection closed; graceful shutdown is unavailable");
        }
    }

    fn on_qmp_event(&self, e: &QmpEvent) {
        let field = |k: &str| match e.data.get(k) {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => "<nil>".into(),
        };
        match e.name.as_str() {
            "BLOCK_IO_ERROR" => {
                if e.data.get("action").and_then(|v| v.as_str()) != Some("stop") {
                    return;
                }
                let mut msg = format!(
                    "I/O error on disk {} ({})",
                    field("device"),
                    field("operation")
                );
                if e.data.get("nospace").and_then(serde_json::Value::as_bool) == Some(true) {
                    msg += ": out of space on the host";
                }
                self.0.st.lock().io_error.clone_from(&msg);
                self.set_state(VmState::Paused, msg);
            }
            "STOP" => {
                let mut msg = self.0.st.lock().io_error.clone();
                if msg.is_empty() {
                    msg = "QEMU paused the guest".into();
                }
                self.set_state(VmState::Paused, msg);
            }
            "RESUME" => self.set_state(VmState::Running, ""),
            "SHUTDOWN" | "POWERDOWN" | "RESET" => self.log(format!("[qmp] {}", e.name)),
            _ => {}
        }
    }

    /// Asks the guest to shut down; the state changes only when QEMU exits.
    pub async fn powerdown(&self) -> Result<()> {
        let q = self
            .qmp()
            .ok_or_else(|| Error::msg("QMP control unavailable: use Force stop"))?;
        q.system_powerdown().await
    }

    /// Ends QEMU at once (like pulling the plug).
    pub fn kill(&self) -> Result<()> {
        let mut st = self.0.st.lock();
        let Some(k) = st.kill.clone() else {
            return Err(Error::msg("the VM is not running"));
        };
        st.killed = true;
        k.notify_one();
        Ok(())
    }
}

/// The base of the private run directories: `$XDG_RUNTIME_DIR`, or the temporary directory.
pub fn runtime_base() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|r| !r.is_empty())
        .map_or_else(std::env::temp_dir, PathBuf::from)
}

/// Creates a private (0700) run directory `macos-kvm-gui-*` under `base`, as Go's
/// `os.MkdirTemp(base, "macos-kvm-gui-")` does, so both apps find each other's sockets.
fn private_run_dir(base: &Path) -> Result<PathBuf> {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for i in 0..100u128 {
        let n = (seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(i)
            ^ u128::from(std::process::id()))
            % 1_000_000_000;
        let p = base.join(format!("{RUN_DIR_PREFIX}{n}"));
        match std::fs::DirBuilder::new().mode(0o700).create(&p) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    Err(Error::msg("could not create a run directory"))
}

/// Go's `exec.ExitError` text: `None` for success, `exit status N` or `signal: N`.
fn exit_error(s: ExitStatus) -> Option<String> {
    if s.success() {
        return None;
    }
    Some(match (s.code(), s.signal()) {
        (Some(c), _) => format!("exit status {c}"),
        (None, Some(sig)) => format!("signal: {sig}"),
        _ => s.to_string(),
    })
}

/// QEMU's last output lines as a message for the user (`err` is `None` for a normal exit,
/// which gives "" unless the output names a known problem).
pub fn explain_exit(err: Option<&str>, last_lines: &[String]) -> String {
    let out = last_lines.join("\n");
    let has = |s: &str| out.contains(s);
    if has("failed to initialize kvm: Device or resource busy") {
        return "KVM is busy: another hypervisor (VirtualBox or VMware) is using virtualization. Shut its VM down and try again.".into();
    }
    if has(r#"Failed to get "write" lock"#) {
        return "The disk is already open in another QEMU (started outside the app?). Shut that VM down first.".into();
    }
    if has("Could not set up host forwarding rule") {
        return "The SSH port is already in use on 127.0.0.1 (another VM?). Choose another port in Settings › Network.".into();
    }
    if has("bridge helper")
        || has("access denied by acl file")
        || has("failed to create tun device")
        || has("qemu-bridge-helper")
    {
        return "A bridged adapter could not attach: qemu-bridge-helper needs to be setuid root (or hold CAP_NET_ADMIN) and the bridge must be allowed in /etc/qemu/bridge.conf. See Troubleshooting › Bridged networking.".into();
    }
    let Some(err) = err else {
        return String::new();
    };
    if let Some(m) = rg_host::Host::system().udmabuf_exit(&out) {
        return m;
    }
    if has("Could not access KVM kernel module") || has("/dev/kvm") {
        return "No access to KVM. Check the Host tab.".into();
    }
    // Warnings such as the "pcid" one do not explain the exit.
    let last = last_lines
        .iter()
        .rev()
        .find(|l| !l.contains("warning:"))
        .map_or(String::new(), |l| format!(": {l}"));
    format!("QEMU exited with an error ({err}){last}")
}

/// The soft RLIMIT_MEMLOCK in bytes (VFIO pins all guest RAM); `u64::MAX` when unlimited.
pub fn memlock_limit() -> u64 {
    rustix::process::getrlimit(rustix::process::Resource::Memlock)
        .current
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn explain_exit_messages() {
        let busy = lines(&[
            "ioctl(KVM_CREATE_VM) failed: 16 Device or resource busy",
            "qemu-system-x86_64: failed to initialize kvm: Device or resource busy",
        ]);
        assert!(explain_exit(Some("exit status 1"), &busy).contains("another hypervisor"));
        let locked = lines(&[
            r#"qemu-system-x86_64: -device ide-hd,bus=sata.4,drive=MacHDD: Failed to get "write" lock"#,
        ]);
        assert!(explain_exit(Some("exit status 1"), &locked).contains("another QEMU"));
        assert_eq!(explain_exit(None, &lines(&["all good"])), "");
        assert_eq!(
            explain_exit(
                Some("exit status 1"),
                &lines(&["boom", "qemu: warning: pcid"])
            ),
            "QEMU exited with an error (exit status 1): boom"
        );
        assert!(
            explain_exit(Some("exit status 1"), &lines(&["udmabuf: open failed"]))
                .contains("modprobe udmabuf")
        );
    }

    #[test]
    fn exit_status_reads_like_go() {
        assert_eq!(exit_error(ExitStatus::from_raw(0)), None);
        assert_eq!(
            exit_error(ExitStatus::from_raw(2 << 8)).as_deref(),
            Some("exit status 2")
        );
        assert_eq!(
            exit_error(ExitStatus::from_raw(9)).as_deref(),
            Some("signal: 9")
        );
    }

    #[test]
    fn memlock_limit_reads_proc() {
        assert!(memlock_limit() > 0);
    }

    #[test]
    fn run_dir_is_private_and_named_for_both_apps() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir()?;
        let d = private_run_dir(t.path())?;
        assert!(
            d.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(RUN_DIR_PREFIX))
        );
        assert_eq!(std::fs::metadata(&d)?.permissions().mode() & 0o777, 0o700);
        Ok(())
    }

    fn recorder() -> (Vm, Arc<Mutex<Vec<VmEvent>>>) {
        let ev = Arc::new(Mutex::new(Vec::new()));
        let e2 = Arc::clone(&ev);
        (Vm::new(Handle::current(), move |e| e2.lock().push(e)), ev)
    }

    fn states(ev: &Mutex<Vec<VmEvent>>) -> Vec<(VmState, String)> {
        ev.lock()
            .iter()
            .filter_map(|e| match e {
                VmEvent::State(s, m) => Some((*s, m.clone())),
                VmEvent::Log(_) => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn qmp_events_pause_and_resume() {
        let (vm, ev) = recorder();
        let e = |name: &str, data| QmpEvent {
            name: name.into(),
            data,
        };
        vm.on_qmp_event(&e(
            "BLOCK_IO_ERROR",
            json!({"action": "report", "device": "MacHDD"}),
        ));
        assert!(states(&ev).is_empty(), "action=report does not pause");
        vm.on_qmp_event(&e(
            "BLOCK_IO_ERROR",
            json!({"action": "stop", "device": "MacHDD", "operation": "write", "nospace": true}),
        ));
        vm.on_qmp_event(&e("STOP", serde_json::Value::Null));
        vm.on_qmp_event(&e("RESUME", serde_json::Value::Null));
        let s = states(&ev);
        assert_eq!(
            s.iter().map(|x| x.0).collect::<Vec<_>>(),
            [VmState::Paused, VmState::Paused, VmState::Running]
        );
        assert_eq!(
            s[0].1,
            "I/O error on disk MacHDD (write): out of space on the host"
        );
        assert!(
            s[1].1.contains("out of space"),
            "STOP after a disk error keeps the cause"
        );
    }

    #[tokio::test]
    async fn start_refuses_without_disk() -> Result<()> {
        let t = tempfile::tempdir()?;
        let (vm, _) = recorder();
        let spec = StartSpec {
            config: Config {
                workspace: t.path().to_string_lossy().into(),
                ..Config::defaults()
            },
            qemu: "/bin/false".into(),
            ..StartSpec::default()
        };
        match vm.start(spec) {
            Err(e) => assert!(e.to_string().contains(DISK_FILE), "{e}"),
            Ok(s) => panic!("must not start without a disk: {s:?}"),
        }
        Ok(())
    }

    /// The runtime end to end with /bin/sh in QEMU's place: it rejects QEMU's arguments and
    /// exits 2, which must come back as a Stopped event with the explanation, the run
    /// directory removed and `on_exit` called.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_runs_and_reports_the_exit() -> Result<()> {
        let t = tempfile::tempdir()?;
        std::fs::write(t.path().join(DISK_FILE), b"")?;
        let (vm, ev) = recorder();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let spec = StartSpec {
            config: Config {
                workspace: t.path().to_string_lossy().into(),
                ..Config::defaults()
            },
            qemu: "/bin/sh".into(),
            on_exit: Some(Box::new(move |_| {
                let _ = done_tx.send(());
            })),
            ..StartSpec::default()
        };
        let vm2 = vm.clone();
        let s = tokio::task::spawn_blocking(move || vm2.start(spec))
            .await
            .map_err(|e| Error::msg(e.to_string()))??;
        assert!(s.pid > 0 && s.vnc_socket.is_some());
        tokio::time::timeout(Duration::from_secs(10), done_rx)
            .await
            .map_err(|e| Error::msg(e.to_string()))?
            .ok();
        for _ in 0..50 {
            if vm.state() == VmState::Stopped {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let st = states(&ev);
        assert_eq!(st.first().map(|x| x.0), Some(VmState::Running));
        let last = st.last().cloned().unwrap_or_default();
        assert_eq!(last.0, VmState::Stopped);
        assert!(
            last.1
                .starts_with("QEMU exited with an error (exit status 2)"),
            "{}",
            last.1
        );
        assert!(
            !s.qmp_socket.parent().is_some_and(Path::exists),
            "run directory removed"
        );
        assert!(vm.kill().is_err(), "not running any more");
        Ok(())
    }
}

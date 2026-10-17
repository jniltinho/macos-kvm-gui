//! The command line (port of the Go app's `cmd/`): the same commands, flags, output and exit
//! status (0, or 1 with `Error: <reason>` on stderr).

use std::cell::RefCell;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};
use rg_core::config::{GRAPHICS_SVGA, load_config, save_config, wide_resolution};
use rg_core::network::{MAX_NICS, new_networks, random_mac};
use rg_core::registry::{export_vm, find_vm, port_warning};
use rg_core::workspace::MEDIA_FILE;
use rg_core::{Config, Registry, VmEntry, Workspace};
use rg_qemu::{VmEvent, VmState, find_running};

use crate::{Ctx, KextsSnapshot, go_duration};

/// Output streams of one command (stdout, stderr).
pub type Out<'a> = &'a mut (dyn Write + Send);

const NOT_TRUE_FALSE: &str = "is not true or false";

#[derive(Parser, Debug)]
#[command(
    name = "reims-gui",
    version,
    about = "Install and run macOS on Linux KVM (home lab use)",
    long_about = "Reims GUI installs and runs macOS on Linux KVM, with OpenCore and OVMF built in.\n\
Without a command it opens the desktop app; the commands below automate the same steps.\n\n\
For home lab use: macOS's license only allows it on Apple-branded hardware."
)]
pub struct Cli {
    /// use this VM settings file instead of the VM list
    #[arg(long, global = true, value_name = "file.json")]
    pub config: Option<PathBuf>,
    /// VM to act on, by name or id (default: the selected VM)
    #[arg(long, global = true, value_name = "name")]
    pub vm: Option<String>,
    /// Check the binary and exit (packaging tests)
    #[arg(long, hide = true)]
    pub self_test: bool,
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Check that the host can run macOS on KVM
    #[command(
        long_about = "Runs the Host tab's checks. Exits with an error when any check fails, so scripts can stop early."
    )]
    Check,
    /// Run the MCP server for AI agents (stdio): screen, keys, pointer, start/stop, offline install
    #[command(
        long_about = "Runs a Model Context Protocol server on standard input and output, for AI agents such as
Claude Code or Codex. Register it once, for example:

  claude mcp add reims-gui -- reims-gui mcp
  codex mcp add reims-gui -- reims-gui mcp

It opens no network port. Agents can see a VM's screen, type and click in it, start and stop it
and install macOS from the offline installer; tools that lose data or accept Apple's license
need explicit flags, which only the user should decide."
    )]
    Mcp,
    /// Set up the host for KVM (asks for your password once)
    #[command(
        long_about = "Installs missing QEMU packages, loads the KVM module, adds you to the kvm group and
enables ignore_msrs, through pkexec. The libvirt and input groups are not needed."
    )]
    Setup,
    /// Show or change the VM settings
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Show or set the Workspace (one folder per VM inside it), or create the selected VM's folder
    Workspace {
        #[command(subcommand)]
        cmd: WorkspaceCmd,
    },
    /// Download, verify and convert the macOS recovery into the workspace
    Download {
        /// macOS version (default: the configured one), e.g. sonoma
        #[arg(long)]
        version: Option<String>,
        /// download Apple's full installer (Big Sur and later, 12-18 GB) into the shared cache instead of the recovery
        #[arg(long)]
        full: bool,
    },
    /// Manage the cached full macOS installers (download them with download --full)
    Installer {
        #[command(subcommand)]
        cmd: InstallerCmd,
    },
    /// Manage the macOS disk
    Disk {
        #[command(subcommand)]
        cmd: DiskCmd,
    },
    /// Manage the install media
    Media {
        #[command(subcommand)]
        cmd: MediaCmd,
    },
    /// Run the macOS VM
    Vm {
        #[command(subcommand)]
        cmd: VmCmd,
    },
    /// GPU passthrough (experimental, untested on real hardware)
    Gpu {
        #[command(subcommand)]
        cmd: GpuCmd,
    },
    /// Manage settings.yaml (theme, boot files, extra kexts)
    Settings {
        #[command(subcommand)]
        cmd: SettingsCmd,
    },
    /// Install, build and inspect Reims vGPU's QEMU (experimental)
    Reims {
        #[command(subcommand)]
        cmd: ReimsCmd,
    },
    /// Show the app version and the built-in boot files
    Version,
    /// Open the desktop app (the default without a command)
    Gui,
    /// MCP vm_ssh's password helper: ssh runs it as SSH_ASKPASS (design D6)
    #[command(hide = true)]
    SshAskpass {
        /// ssh's prompt (not used)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        prompt: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigCmd {
    /// Print the settings as JSON
    Show,
    /// Change settings (workspace, version, ram, cores, vcpus, ssh-port, display, gpu, graphics, resolution, root-patch, nic<N>.enabled|mode|bridge|model|mac)
    #[command(after_help = "Examples:
  reims-gui config set version=sonoma ram=16384 cores=8 vcpus=16 display=app
  reims-gui config set nic2.enabled=true nic2.mode=bridged nic2.bridge=br0 nic2.model=vmxnet3 nic2.mac=random")]
    Set {
        #[arg(value_name = "key=value", required = true)]
        pairs: Vec<String>,
        /// confirm the root patch was reverted in OCLP, to turn root-patch mode off
        #[arg(long)]
        root_patch_reverted: bool,
    },
    /// Set CPU and RAM to the values recommended for this host
    Recommend,
}

#[derive(Subcommand, Debug)]
pub enum WorkspaceCmd {
    /// Print the Workspace
    Show,
    /// Set the Workspace; new and imported VMs get <dir>/<name>. Existing VMs are not moved
    Set { dir: String },
    /// Create the selected VM's folder and extract the built-in OpenCore and OVMF
    Create,
}

#[derive(Subcommand, Debug)]
pub enum InstallerCmd {
    /// List the cached full installers
    List,
    /// Delete a cached full installer
    Remove { build: String },
}

#[derive(Subcommand, Debug)]
pub enum DiskCmd {
    /// Create the thin qcow2 disk (never overwrites an existing one)
    Create {
        /// virtual size in GiB (minimum 40)
        #[arg(long, default_value_t = 80)]
        size: u64,
    },
}

#[derive(Subcommand, Debug)]
pub enum MediaCmd {
    /// Delete BaseSystem.img once macOS boots from the disk
    Remove {
        /// confirm that macOS boots from the disk
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum VmCmd {
    /// List the VMs (the selected one is marked with *)
    List,
    /// Show whether a VM (default: the selected one) is running, and how to reach it
    Status { name: Option<String> },
    /// Create a VM with CPU and RAM recommended for this host, select it and prepare its folder
    #[command(after_help = "Examples:
  reims-gui vm create \"Tahoe lab\" --version tahoe --folder ~/macOS-VMs/tahoe")]
    Create {
        name: String,
        /// macOS version (high-sierra ... tahoe)
        #[arg(long, default_value = "sonoma")]
        version: String,
        /// VM folder (default <Workspace>/<name>)
        #[arg(long)]
        folder: Option<String>,
        /// SSH port on 127.0.0.1 (default: the first free one from 2222; another VM's port
        /// gives the next free one)
        #[arg(long)]
        ssh_port: Option<u16>,
    },
    /// Select the VM that commands (and the app) act on
    Select { name: String },
    /// Remove a VM from the list; --purge also deletes its disk, media and boot files
    Delete {
        name: String,
        /// also delete the disk, install media and boot files the app created in the VM folder
        #[arg(long)]
        purge: bool,
        /// confirm
        #[arg(long)]
        yes: bool,
    },
    /// Write a VM's settings as JSON, or with --full a .tar bundle with everything it needs
    #[command(after_help = "Examples:
  reims-gui vm export \"macOS Sonoma\" sonoma.json
  reims-gui vm export \"macOS Sonoma\" sonoma.tar --full")]
    Export {
        name: String,
        file: Option<PathBuf>,
        /// write a .tar bundle with the disk, NVRAM, identity and install media
        #[arg(long)]
        full: bool,
    },
    /// Add a VM from a settings file or a full bundle (unpacked into --folder)
    Import {
        #[arg(value_name = "file.json|file.tar")]
        file: PathBuf,
        /// VM folder for a .tar bundle (default <Workspace>/<name>)
        #[arg(long)]
        folder: Option<String>,
    },
    /// Move a VM's disk, NVRAM and boot files to a new folder (default <Workspace>/<name>)
    Move {
        name: String,
        folder: Option<String>,
    },
    /// Start the VM and wait until it stops
    #[command(
        long_about = "Starts the VM in the foreground and returns when QEMU exits.
Ctrl+C asks macOS to shut down (ACPI); a second Ctrl+C forces the stop.
With display=app there is no window: the screen is on the printed private VNC socket."
    )]
    Start {
        /// app (VNC socket, no window) or window (QEMU window); default from config
        #[arg(long)]
        display: Option<String>,
        /// print the QEMU command line and output
        #[arg(short, long)]
        verbose: bool,
        /// show macOS's boot log on screen instead of the logo, for this start only (boot-args -v debug=0x100)
        #[arg(long)]
        verbose_boot: bool,
        /// The MCP server's kexts snapshot (JSON), so its start uses what it downloaded.
        #[arg(long, hide = true)]
        kexts_snapshot: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum GpuCmd {
    /// List display adapters and whether they can be passed to the VM
    List,
    /// Reserve an eligible GPU for vfio-pci (needs a host reboot)
    Reserve {
        #[arg(value_name = "pci-address")]
        addr: String,
    },
    /// Remove the vfio-pci reservation (needs a host reboot)
    Release,
}

#[derive(Subcommand, Debug)]
pub enum SettingsCmd {
    /// Show settings.yaml's path and the settings in effect
    Show,
    /// Overwrite settings.yaml with the default, which sets nothing
    Reset {
        /// confirm overwriting settings.yaml
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ReimsCmd {
    /// Download the prebuilt Reims vGPU QEMU for this host (Ubuntu 24.04/26.04, Debian 13, Rocky Linux 10) and activate it
    Install,
    /// Build Reims vGPU's QEMU from source in Docker and activate it
    #[command(
        long_about = "Build Reims vGPU's QEMU from source, at the commit the app pins, in an Ubuntu container,
caching the build in <Workspace>/.reims. A build for another release than the host's stays
staged, not activated."
    )]
    Build {
        /// Ubuntu release to build for, 24.04 or 26.04 (default: the host's)
        #[arg(long)]
        release: Option<String>,
    },
    /// Show the Reims vGPU QEMU a reims-vgpu VM runs
    Status,
}

/// Runs the binary: the GUI without a command (or with `gui`), else the command. `gui` gets
/// `--config`'s file. Parse errors and failures exit with 1, like the Go app.
pub fn main(
    args: impl IntoIterator<Item = OsString>,
    ctx: &Ctx,
    gui: &dyn Fn(Option<&Path>) -> Result<()>,
) -> ExitCode {
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    let res = if cli.self_test {
        crate::self_test(ctx.clone()).map(|tools| println!("self-test: {} MCP tools", tools.len()))
    } else if matches!(cli.cmd, None | Some(Cmd::Gui)) {
        gui(cli.config.as_deref())
    } else {
        run(cli, ctx, &mut std::io::stdout(), &mut std::io::stderr())
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Parses `args` (without the program name) and runs the command, writing to `out` and `err`.
pub fn execute(args: &[&str], ctx: &Ctx, out: Out<'_>, err: Out<'_>) -> Result<()> {
    let cli = Cli::try_parse_from(std::iter::once("reims-gui").chain(args.iter().copied()))?;
    run(cli, ctx, out, err)
}

fn run(cli: Cli, ctx: &Ctx, out: Out<'_>, err: Out<'_>) -> Result<()> {
    let Some(cmd) = cli.cmd else {
        bail!("no command");
    };
    let c = Command {
        ctx,
        config: cli.config,
        vm: cli.vm,
    };
    match cmd {
        Cmd::Check => c.check(out),
        Cmd::Mcp => crate::serve_stdio(ctx.clone()),
        Cmd::Setup => c.setup(out),
        Cmd::Config { cmd } => match cmd {
            ConfigCmd::Show => c.config_show(out),
            ConfigCmd::Set {
                pairs,
                root_patch_reverted,
            } => c.config_set(&pairs, root_patch_reverted, out, err),
            ConfigCmd::Recommend => c.config_recommend(out),
        },
        Cmd::Workspace { cmd } => match cmd {
            WorkspaceCmd::Show => Ok(writeln!(out, "{}", ctx.registry()?.workspace_root())?),
            WorkspaceCmd::Set { dir } => {
                let reg = ctx.registry()?;
                reg.set_workspace_root(&dir)?;
                Ok(writeln!(out, "workspace: {}", reg.workspace_root())?)
            }
            WorkspaceCmd::Create => {
                let e = c.current_vm()?;
                ctx.ops.prepare_workspace(&e.config)?;
                Ok(writeln!(
                    out,
                    "VM folder ready: {}",
                    e.config.workspace_dir()?.display()
                )?)
            }
        },
        Cmd::Download { version, full } => c.download(version, full, out),
        Cmd::Installer { cmd } => c.installer(cmd, out),
        Cmd::Disk {
            cmd: DiskCmd::Create { size },
        } => {
            let w = Workspace::new(c.current_vm()?.config.workspace_dir()?);
            if let Some(warning) = w.create_disk(size)? {
                writeln!(err, "warning: {warning}")?;
            }
            Ok(writeln!(out, "disk created: {size} GiB")?)
        }
        Cmd::Media {
            cmd: MediaCmd::Remove { yes },
        } => {
            if !yes {
                bail!("pass --yes to confirm that macOS already boots from the installed disk");
            }
            let freed = Workspace::new(c.current_vm()?.config.workspace_dir()?).remove_media()?;
            Ok(writeln!(
                out,
                "install media removed: {:.2} GiB freed",
                freed as f64 / (1u64 << 30) as f64
            )?)
        }
        Cmd::Vm { cmd } => c.vm(cmd, out, err),
        Cmd::Gpu { cmd } => c.gpu(cmd, out),
        Cmd::Settings { cmd } => c.settings(cmd, out),
        Cmd::Reims { cmd } => c.reims(cmd, out),
        Cmd::Version => {
            let dir = ctx.app.setting(rg_core::settings::MKG_ASSETS_DIR);
            writeln!(out, "reims-gui {}", ctx.version)?;
            let dir = (!dir.is_empty()).then(|| PathBuf::from(dir));
            Ok(writeln!(
                out,
                "boot files: {}",
                rg_assets::asset_versions(dir.as_deref())
            )?)
        }
        Cmd::Gui => bail!("gui is handled by the binary"),
        Cmd::SshAskpass { .. } => crate::ssh::askpass(out),
    }
}

/// Go's tabwriter (padding 2): every cell but a line's last is padded to its column's width.
fn table(rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = Vec::new();
    for r in rows {
        for (i, cell) in r.iter().enumerate().take(r.len().saturating_sub(1)) {
            if widths.len() <= i {
                widths.push(0);
            }
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let mut s = String::new();
    for r in rows {
        for (i, cell) in r.iter().enumerate() {
            s.push_str(cell);
            if i + 1 < r.len() {
                s.push_str(&" ".repeat(widths[i] - cell.chars().count() + 2));
            }
        }
        s.push('\n');
    }
    s
}

fn no_vm(name: &str) -> anyhow::Error {
    anyhow!("no VM named {name:?} (see `vm list`)")
}

/// "running (pid N, up 1h2m3s)" or "stopped"; VMs started outside the app say so.
fn status_of(c: &Config) -> String {
    let Some(r) = find_running(c) else {
        return "stopped".into();
    };
    let mut s = format!("running (pid {}, up {})", r.pid, go_duration(r.uptime()));
    if !r.by_app {
        s += ", outside the app";
    }
    s
}

fn parse_bool(v: &str) -> Result<bool, String> {
    match v {
        "1" | "t" | "T" | "TRUE" | "true" | "True" => Ok(true),
        "0" | "f" | "F" | "FALSE" | "false" | "False" => Ok(false),
        _ => Err(format!("{v:?} {NOT_TRUE_FALSE}")),
    }
}

fn parse_num<T: std::str::FromStr>(v: &str) -> Result<T, String> {
    v.parse()
        .map_err(|_| format!("{v:?} is not a whole number"))
}

/// Applies one `config set` key; `None` for an unknown key.
pub(crate) fn set_key(c: &mut Config, k: &str, v: &str) -> Option<Result<(), String>> {
    let s = v.to_string();
    Some(match k {
        "workspace" => {
            c.workspace = s;
            Ok(())
        }
        "version" => {
            c.version = s;
            Ok(())
        }
        "display" => {
            c.display = s;
            Ok(())
        }
        "gpu" => {
            c.gpu = s;
            Ok(())
        }
        "graphics" => {
            c.graphics = s;
            Ok(())
        }
        "resolution" => {
            c.resolution = s;
            Ok(())
        }
        "ram" => parse_num(v).map(|n| c.ram_mib = n),
        "cores" => parse_num(v).map(|n| c.cores = n),
        "vcpus" => parse_num(v).map(|n| c.vcpus = n),
        "ssh-port" => parse_num(v).map(|n| c.ssh_port = n),
        "root-patch" => parse_bool(v).map(|b| c.root_patch = b),
        _ => {
            // nic<N>.<field>, N in 1..=MAX_NICS
            let (n, field) = k.strip_prefix("nic")?.split_once('.')?;
            let n: usize = n.parse().ok().filter(|n| (1..=MAX_NICS).contains(n))?;
            if !matches!(field, "enabled" | "mode" | "bridge" | "model" | "mac") {
                return None;
            }
            let nic = match c.nic_mut(n - 1) {
                Ok(nic) => nic,
                Err(e) => return Some(Err(e.to_string())),
            };
            match field {
                "enabled" => parse_bool(v).map(|b| nic.enabled = b),
                "mode" => {
                    nic.mode = s;
                    Ok(())
                }
                "bridge" => {
                    nic.bridge = s;
                    Ok(())
                }
                "model" => {
                    nic.model = s;
                    Ok(())
                }
                _ => {
                    nic.mac = if v == "random" {
                        random_mac()
                    } else {
                        v.to_lowercase()
                    };
                    Ok(())
                }
            }
        }
    })
}

/// One command's context: the global flags.
struct Command<'a> {
    ctx: &'a Ctx,
    config: Option<PathBuf>,
    vm: Option<String>,
}

impl Command<'_> {
    /// The VM the command acts on: `--config`, then `--vm`, then the selected VM. With no VM
    /// yet, it creates "macOS Sonoma" with the defaults, like the app.
    fn current_vm(&self) -> Result<VmEntry> {
        if let Some(p) = &self.config {
            return Ok(VmEntry {
                id: "config".into(),
                path: Some(p.clone()),
                config: load_config(p)?,
            });
        }
        let reg = self.ctx.registry()?;
        let vms = reg.list()?;
        if vms.is_empty() {
            let mut c = Config::defaults();
            c.name = "macOS Sonoma".into();
            c.workspace = reg.vm_folder(&c.name);
            c.networks = Some(new_networks(&c.version));
            return Ok(reg.create(c)?);
        }
        if let Some(k) = &self.vm {
            return find_vm(&vms, k).cloned().ok_or_else(|| no_vm(k));
        }
        reg.selected(&vms).ok_or_else(|| anyhow!("no VM selected"))
    }

    fn save_vm(&self, e: &VmEntry, c: Config) -> Result<()> {
        match &e.path {
            Some(p) => save_config(p, &c)?,
            None => self.ctx.registry()?.save(e, c)?,
        }
        Ok(())
    }

    fn check(&self, out: Out<'_>) -> Result<()> {
        let c = self.current_vm()?.config;
        let mut rows = Vec::new();
        let mut failed = 0;
        for ch in self.ctx.host().checks_for(&c) {
            let mut last = ch.detail.clone();
            if !ch.hint.is_empty() {
                last += &format!(" ({})", ch.hint);
            }
            if ch.status == rg_host::Status::Fail {
                failed += 1;
            }
            rows.push(vec![ch.status.to_string(), ch.name, last]);
        }
        write!(out, "{}", table(&rows))?;
        if failed > 0 {
            bail!("{failed} check(s) failed; `reims-gui setup` fixes the common ones");
        }
        Ok(())
    }

    fn setup(&self, out: Out<'_>) -> Result<()> {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .map_err(|_| anyhow!("cannot tell who you are: $USER is not set"))?;
        let msg = self.ctx.host().setup_host(&user)?;
        Ok(writeln!(out, "{msg}")?)
    }

    fn config_show(&self, out: Out<'_>) -> Result<()> {
        let e = self.current_vm()?;
        let mut c = e.config.clone();
        if c.graphics.is_empty() {
            c.graphics = GRAPHICS_SVGA.into(); // the default, printed so it shows
        }
        c.resolution = c.reims_resolution().into(); // used by Reims vGPU only
        let src = match &e.path {
            Some(p) => p.display().to_string(),
            None => format!("{} (VM {})", self.ctx.app.db_path().display(), e.id),
        };
        Ok(writeln!(
            out,
            "# {src}\n{}",
            serde_json::to_string_pretty(&c)?
        )?)
    }

    fn config_set(
        &self,
        pairs: &[String],
        reverted: bool,
        out: Out<'_>,
        err: Out<'_>,
    ) -> Result<()> {
        let e = self.current_vm()?;
        let mut c = e.config.clone();
        for a in pairs {
            let applied = a
                .split_once('=')
                .and_then(|(k, v)| set_key(&mut c, k, v).map(|r| (k, r)));
            match applied {
                None => bail!("expected key=value with a known key, got {a:?}"),
                Some((k, Err(err))) => bail!("{k}: {err}"),
                Some((_, Ok(()))) => {}
            }
        }
        if c.root_patch
            && (!e.config.root_patch || !rg_assets::root_patch_kexts_ready(&self.ctx.kext_cache()?))
        {
            // Also when the mode was already on but the kexts are missing (an imported VM, a
            // moved Workspace).
            c.validate()?;
            writeln!(
                out,
                "root-patch mode lowers SIP to 0x803 and loads AMFIPass, RSRHelper, RestrictEvents and KDKlessWorkaround;"
            )?;
            writeln!(
                out,
                "downloading them from OpenCore Legacy Patcher 2.5.1 (checked against their SHA-256)..."
            )?;
            rg_assets::download_root_patch_kexts(&self.ctx.kext_cache()?, &AtomicBool::new(false))?;
        } else if !c.root_patch && e.config.root_patch && !reverted {
            bail!(
                "revert the root patch in OCLP inside macOS first (Post-Install Root Patch › Revert Root Patches) and restart: \
remaining root patches can prevent macOS from booting once its protections are restored. Then run this again with --root-patch-reverted"
            );
        }
        let warn = c.reims() && wide_resolution(c.reims_resolution()) && !c.reims_kext();
        let port = pairs
            .iter()
            .any(|a| a.starts_with("ssh-port="))
            .then_some(c.ssh_port);
        self.save_vm(&e, c)?;
        if let Some(port) = port
            && e.path.is_none()
            && let Some(w) = port_warning(&self.ctx.registry()?.ssh_port_users(port, &e.id)?)
        {
            writeln!(err, "warning: {w}")?;
        }
        if warn {
            writeln!(
                out,
                "warning: between 2048 and 3840 px wide, WindowServer crashed at every login on macOS Sequoia (ReimsVgpu.kext, which fixes it, loads on macOS 26 only); 3840x2160 works as a HiDPI desktop"
            )?;
        }
        Ok(writeln!(out, "saved {}", e.display_name())?)
    }

    fn config_recommend(&self, out: Out<'_>) -> Result<()> {
        let e = self.current_vm()?;
        let c = e
            .config
            .clone()
            .recommended(threads(), self.ctx.host().mem_total_mib());
        let (ram, vcpus, cores) = (c.ram_mib, c.vcpus, c.cores);
        self.save_vm(&e, c)?;
        Ok(writeln!(
            out,
            "saved {}: {ram} MiB, {vcpus} vCPUs ({cores} cores)",
            e.display_name()
        )?)
    }

    fn download(&self, version: Option<String>, full: bool, out: Out<'_>) -> Result<()> {
        let c = self.current_vm()?.config;
        let version = version
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| c.version.clone());
        let out = RefCell::new(out);
        let (mut stage, mut last) = (String::new(), None::<Instant>);
        let mut progress = |p: rg_macos::Progress| {
            let mut o = out.borrow_mut();
            if p.phase != stage {
                stage.clone_from(&p.phase);
                let _ = write!(o, "\n{stage}");
            }
            if let Some(pct) = p.pct()
                && last.is_none_or(|t| t.elapsed() > Duration::from_millis(500))
            {
                last = Some(Instant::now());
                let _ = write!(o, "\r{stage}: {pct:.1}%");
            }
        };
        let mut log = |s: &str| {
            let _ = writeln!(out.borrow_mut(), "{s}");
        };
        let cancel = AtomicBool::new(false);
        if full {
            let dir = rg_macos::installers_dir(&self.ctx.workspace_root()?);
            let res = rg_macos::download_installer(
                &rg_macos::Http::default(),
                &dir,
                &version,
                &cancel,
                &mut progress,
                &mut log,
            );
            let mut o = out.borrow_mut();
            writeln!(o)?;
            let ci = res?;
            let p = &ci.product;
            return Ok(writeln!(
                o,
                "full installer ready: macOS {} ({}), {}",
                p.version,
                p.build,
                ci.path.display()
            )?);
        }
        let res = self
            .ctx
            .ops
            .download_media(&c, &version, &cancel, &mut progress, &mut log);
        let mut o = out.borrow_mut();
        if let Err(e) = res {
            writeln!(o)?;
            return Err(e);
        }
        let media = c.workspace_dir()?.join(MEDIA_FILE);
        Ok(writeln!(
            o,
            "\ninstall media ready: {} ({version})",
            media.display()
        )?)
    }

    fn installer(&self, cmd: InstallerCmd, out: Out<'_>) -> Result<()> {
        let dir = rg_macos::installers_dir(&self.ctx.workspace_root()?);
        match cmd {
            InstallerCmd::List => {
                let mut rows = vec![vec![
                    "VERSION".into(),
                    "BUILD".into(),
                    "SIZE".into(),
                    "PATH".into(),
                ]];
                for c in rg_macos::cached_installers(&dir) {
                    let p = &c.product;
                    rows.push(vec![
                        p.version.clone(),
                        p.build.clone(),
                        format!("{:.1} GB", p.size as f64 / 1e9),
                        c.path.display().to_string(),
                    ]);
                }
                Ok(write!(out, "{}", table(&rows))?)
            }
            InstallerCmd::Remove { build } => {
                rg_macos::remove_installer(&dir, &build)?;
                Ok(writeln!(out, "removed {build}")?)
            }
        }
    }

    fn find(&self, reg: &Registry, name: &str) -> Result<VmEntry> {
        find_vm(&reg.list()?, name)
            .cloned()
            .ok_or_else(|| no_vm(name))
    }

    fn vm(&self, cmd: VmCmd, out: Out<'_>, err: Out<'_>) -> Result<()> {
        let ctx = self.ctx;
        match cmd {
            VmCmd::List => {
                let reg = ctx.registry()?;
                let vms = reg.list()?;
                let sel = reg.selected(&vms).map(|e| e.id).unwrap_or_default();
                let mut rows = vec![
                    ["", "NAME", "STATUS", "VERSION", "RAM", "VCPUS", "FOLDER"]
                        .map(String::from)
                        .to_vec(),
                ];
                for e in &vms {
                    let c = &e.config;
                    rows.push(vec![
                        if e.id == sel {
                            "*".into()
                        } else {
                            String::new()
                        },
                        e.display_name(),
                        status_of(c),
                        c.version.clone(),
                        format!("{} MiB", c.ram_mib),
                        c.vcpus.to_string(),
                        c.workspace.clone(),
                    ]);
                }
                Ok(write!(out, "{}", table(&rows))?)
            }
            VmCmd::Status { name } => {
                let current = self.current_vm();
                let e = match name {
                    Some(n) => self.find(&ctx.registry()?, &n)?,
                    None => current?,
                };
                let c = &e.config;
                writeln!(out, "name:      {} ({})", e.display_name(), e.id)?;
                writeln!(out, "status:    {}", status_of(c))?;
                writeln!(
                    out,
                    "macOS:     {}, {} MiB, {} vCPUs",
                    c.version, c.ram_mib, c.vcpus
                )?;
                writeln!(out, "folder: {}", c.workspace)?;
                if c.ssh_forwarded() {
                    writeln!(out, "ssh:       ssh -p {} user@127.0.0.1", c.ssh_port)?;
                    if e.path.is_none()
                        && let Some(w) =
                            port_warning(&ctx.registry()?.ssh_port_users(c.ssh_port, &e.id)?)
                    {
                        writeln!(out, "warning:   {w}")?;
                    }
                } else {
                    writeln!(
                        out,
                        "ssh:       no 127.0.0.1 forward (no NAT adapter): use the VM's address on its network"
                    )?;
                }
                if let Some(sock) = find_running(c).and_then(|r| r.vnc_socket) {
                    writeln!(out, "screen:    VNC socket {}", sock.display())?;
                }
                Ok(())
            }
            VmCmd::Create {
                name,
                version,
                folder,
                ssh_port,
            } => {
                let reg = ctx.registry()?;
                let mut c = Config::defaults().recommended(threads(), ctx.host().mem_total_mib());
                c.workspace = folder
                    .filter(|f| !f.is_empty())
                    .unwrap_or_else(|| reg.vm_folder(&name));
                c.name = name;
                c.version = version;
                c.ssh_port = ssh_port.unwrap_or(0);
                c.networks = Some(new_networks(&c.version));
                let (e, notice) = ctx.ops.create_vm(&reg, c, None)?;
                if let Some(n) = notice {
                    writeln!(out, "{n}")?;
                }
                let dir = e.config.workspace_dir()?;
                Ok(writeln!(
                    out,
                    "created and selected {} ({}), folder {}, SSH port {}",
                    e.display_name(),
                    e.id,
                    dir.display(),
                    e.config.ssh_port
                )?)
            }
            VmCmd::Select { name } => {
                let reg = ctx.registry()?;
                let e = self.find(&reg, &name)?;
                reg.select(&e.id)?;
                Ok(writeln!(out, "selected {}", e.display_name())?)
            }
            VmCmd::Delete { name, purge, yes } => {
                if !yes {
                    bail!("pass --yes to confirm");
                }
                let reg = ctx.registry()?;
                let e = self.find(&reg, &name)?;
                if purge && find_running(&e.config).is_some() {
                    bail!("shut down the VM before deleting its files");
                }
                for r in reg.delete(&e, purge)? {
                    writeln!(out, "deleted {r}")?;
                }
                Ok(())
            }
            VmCmd::Export { name, file, full } => {
                let e = self.find(&ctx.registry()?, &name)?;
                if full {
                    let Some(dst) = file else {
                        bail!("--full needs a destination file");
                    };
                    if find_running(&e.config).is_some() {
                        bail!("shut down {} before exporting it", e.display_name());
                    }
                    return Ok(rg_core::bundle::export_bundle(
                        &e,
                        &dst,
                        &AtomicBool::new(false),
                        |done, total| {
                            let _ = write!(
                                err,
                                "\rexporting: {:.0}%",
                                done as f64 * 100.0 / total.max(1) as f64
                            );
                        },
                    )?);
                }
                let json = export_vm(&e)?;
                match file {
                    Some(f) => Ok(std::fs::write(f, json + "\n")?),
                    None => Ok(writeln!(out, "{json}")?),
                }
            }
            VmCmd::Import { file, folder } => {
                let reg = ctx.registry()?;
                let is_tar = file
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("tar"));
                let (e, notice) = if is_tar {
                    let ws = folder.filter(|f| !f.is_empty()).unwrap_or_else(|| {
                        reg.vm_folder(
                            &file
                                .file_stem()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        )
                    });
                    let dir = Config {
                        workspace: ws,
                        ..Config::default()
                    }
                    .workspace_dir()?;
                    let res =
                        reg.import_bundle(&file, &dir, &AtomicBool::new(false), |done, total| {
                            let _ = write!(
                                err,
                                "\rimporting: {:.0}%",
                                done as f64 * 100.0 / total.max(1) as f64
                            );
                        });
                    writeln!(err)?;
                    res?
                } else {
                    reg.import_settings(&file)?
                };
                if let Some(n) = notice {
                    writeln!(out, "{n}")?;
                }
                Ok(writeln!(
                    out,
                    "imported {} ({}), folder {}, SSH port {}",
                    e.display_name(),
                    e.id,
                    e.config.workspace,
                    e.config.ssh_port
                )?)
            }
            VmCmd::Move { name, folder } => {
                let reg = ctx.registry()?;
                let e = self.find(&reg, &name)?;
                if find_running(&e.config).is_some() {
                    bail!("shut down the VM before moving it");
                }
                let dst = folder.unwrap_or_else(|| reg.vm_folder(&e.config.name));
                let e = reg.move_vm(&e, &dst)?;
                Ok(writeln!(
                    out,
                    "moved {} to {}",
                    e.display_name(),
                    e.config.workspace
                )?)
            }
            VmCmd::Start {
                display,
                verbose,
                verbose_boot,
                kexts_snapshot,
            } => self.vm_start(
                display,
                verbose,
                verbose_boot,
                kexts_snapshot.as_deref(),
                out,
            ),
        }
    }

    fn vm_start(
        &self,
        display: Option<String>,
        verbose: bool,
        verbose_boot: bool,
        snapshot: Option<&str>,
        out: Out<'_>,
    ) -> Result<()> {
        let ctx = self.ctx;
        let mut c = self.current_vm()?.config;
        if let Some(d) = display.filter(|d| !d.is_empty()) {
            c.display = d;
        }
        c.verbose_boot = verbose_boot;
        let ks = match snapshot {
            Some(s) => KextsSnapshot::parse(s)?, // the MCP server's start, which already downloaded them
            None => ctx.app.load_extra_kexts()?,
        };
        let cache = ctx.kext_cache()?;
        let cancel = AtomicBool::new(false);
        let aks = ks.clone();
        if !rg_assets::extra_kexts_ready(&aks, &cache) {
            writeln!(
                out,
                "vm: downloading the kexts listed in {}",
                ctx.app.settings_file().display()
            )?;
            rg_assets::download_extra_kexts(&aks, &cache, &cancel)?;
        }
        if c.reims_kext() && !rg_assets::reims_kext_ready(&cache) {
            writeln!(out, "vm: downloading ReimsVgpu.kext (Reims vGPU on Tahoe)")?;
            rg_assets::download_reims_kext(&cache, &cancel)?;
        }
        c.extras = Some(ks);

        enum Msg {
            Note(String),
            Stopped(String),
            Signal,
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let events = tx.clone();
        let vm = ctx.ops.start_vm(
            c,
            Box::new(move |ev| {
                let tx = &events;
                let _ = match ev {
                    VmEvent::State(s, msg) => {
                        if !msg.is_empty() {
                            let _ = tx.send(Msg::Note(format!("vm: {msg}")));
                        }
                        if s == VmState::Stopped {
                            tx.send(Msg::Stopped(msg))
                        } else {
                            Ok(())
                        }
                    }
                    VmEvent::Log(line) if verbose => tx.send(Msg::Note(line)),
                    VmEvent::Log(_) => Ok(()),
                };
            }),
        )?;
        writeln!(out, "vm: running; Ctrl+C shuts it down")?;
        if let Some(sock) = vm.vnc_socket() {
            writeln!(out, "vm: screen on VNC socket {}", sock.display())?;
        }
        if let Some(line) = vm.installer_command() {
            writeln!(
                out,
                "vm: the full installer disc is attached; to install from it, type in the recovery's Terminal: {line}"
            )?;
        }
        std::thread::spawn(move || forward_signals(|| tx.send(Msg::Signal).is_ok()));
        let mut asked = false;
        loop {
            match rx.recv() {
                Ok(Msg::Note(s)) => writeln!(out, "{s}")?,
                Ok(Msg::Stopped(msg)) if !msg.is_empty() && !asked => bail!("{msg}"),
                Ok(Msg::Stopped(_)) | Err(_) => return Ok(()),
                Ok(Msg::Signal) if asked => {
                    writeln!(out, "vm: forcing the stop")?;
                    let _ = vm.kill();
                }
                Ok(Msg::Signal) => {
                    asked = true;
                    writeln!(out, "vm: asking macOS to shut down (Ctrl+C again to force)")?;
                    if let Err(e) = vm.powerdown() {
                        writeln!(out, "vm: {e}")?;
                    }
                }
            }
        }
    }

    fn gpu(&self, cmd: GpuCmd, out: Out<'_>) -> Result<()> {
        let h = self.ctx.host();
        match cmd {
            GpuCmd::List => {
                let mut rows = vec![
                    ["ADDRESS", "ID", "NAME", "ELIGIBLE", "REASON"]
                        .map(String::from)
                        .to_vec(),
                ];
                for g in h.gpus() {
                    rows.push(vec![g.addr, g.id, g.name, g.eligible.to_string(), g.reason]);
                }
                Ok(write!(out, "{}", table(&rows))?)
            }
            GpuCmd::Reserve { addr } => {
                let Some(g) = h.gpus().into_iter().find(|g| g.addr == addr) else {
                    bail!("no GPU at {addr} (see `gpu list`)");
                };
                h.reserve_gpu(&g)?;
                Ok(writeln!(
                    out,
                    "reserved {0} for vfio-pci: reboot the host, then `config set gpu={0}`",
                    g.addr
                )?)
            }
            GpuCmd::Release => {
                h.release_gpu()?;
                Ok(writeln!(out, "reservation removed: reboot the host")?)
            }
        }
    }

    fn settings(&self, cmd: SettingsCmd, out: Out<'_>) -> Result<()> {
        let app = &self.ctx.app;
        let file = app.settings_file();
        match cmd {
            SettingsCmd::Show => {
                use rg_core::settings::{MKG_ASSETS_DIR, MKG_REIMS_QEMU, MKG_THEME};
                let ks = app.load_extra_kexts()?;
                writeln!(out, "file: {}", file.display())?;
                writeln!(out, "theme: {}", app.setting(MKG_THEME))?;
                writeln!(out, "assets_dir: {}", app.setting(MKG_ASSETS_DIR))?;
                writeln!(out, "reims_qemu: {}", app.setting(MKG_REIMS_QEMU))?;
                for e in &ks.entries {
                    write!(out, "kext: {}", e.url)?;
                    if !e.kexts.is_empty() {
                        write!(out, " (load {})", e.kexts.join(", "))?;
                    }
                    writeln!(out)?;
                }
                if !ks.disabled.is_empty() {
                    writeln!(out, "disable: {}", ks.disabled.join(", "))?;
                }
                if !ks.entries.is_empty() {
                    let ready = rg_assets::extra_kexts_ready(&ks.clone(), &self.ctx.kext_cache()?);
                    writeln!(out, "kexts downloaded: {ready}")?;
                }
                Ok(())
            }
            SettingsCmd::Reset { yes } => {
                if !yes {
                    bail!("pass --yes to overwrite {}", file.display());
                }
                app.reset_settings()?;
                Ok(writeln!(out, "settings reset: {}", file.display())?)
            }
        }
    }

    fn reims(&self, cmd: ReimsCmd, out: Out<'_>) -> Result<()> {
        let reims = self.ctx.reims()?;
        let cancel = AtomicBool::new(false);
        let out = Mutex::new(out);
        let log = |s: &str| {
            let mut o = out
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _ = writeln!(o, "{s}");
        };
        match cmd {
            ReimsCmd::Install => Ok(reims.install(&cancel, &log)?),
            ReimsCmd::Build { release } => {
                Ok(reims.build(release.as_deref().filter(|r| !r.is_empty()), &cancel, &log)?)
            }
            ReimsCmd::Status => {
                let info = reims.installed()?;
                log(&format!("qemu: {}", info.qemu.display()));
                log(&format!("build: {}", info.build));
                log(&format!("commit: {}", info.commit));
                match info.apple_gfx {
                    Err(e) => log(&format!("apple-gfx-pci: no: {e}")),
                    Ok(()) => log("apple-gfx-pci: yes"),
                }
                Ok(())
            }
        }
    }
}

/// The host's threads (Go's `runtime.NumCPU`).
pub(crate) fn threads() -> u32 {
    std::thread::available_parallelism().map_or(1, |n| u32::try_from(n.get()).unwrap_or(u32::MAX))
}

/// Calls `send` on every SIGINT or SIGTERM until it returns false.
fn forward_signals(send: impl Fn() -> bool) {
    use tokio::signal::unix::{SignalKind, signal};
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    rt.block_on(async {
        let (Ok(mut int), Ok(mut term)) = (
            signal(SignalKind::interrupt()),
            signal(SignalKind::terminate()),
        ) else {
            return;
        };
        loop {
            tokio::select! {
                _ = int.recv() => {}
                _ = term.recv() => {}
            }
            if !send() {
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests;

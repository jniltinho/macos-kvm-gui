//! reims-gui: manage macOS VMs on QEMU/KVM with Reims vGPU. One binary: the GUI, the CLI and
//! the MCP server.

mod agent_ops;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use agent_ops::AgentOps;
use rg_core::{AppDir, Registry};
use rg_ui::{RealBackend, RunOptions};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    // Flags rg-agent's parser does not know, or that must not touch the user's data.
    let flag = |f: &str| std::env::args_os().skip(1).any(|a| a == f);
    if flag("--demo") {
        return exit(rg_ui::run_demo());
    }
    if flag("--self-test") {
        return exit(self_test());
    }
    let Some(app) = AppDir::user() else {
        eprintln!("Error: no config folder ($HOME is not set)");
        return ExitCode::FAILURE;
    };
    let ctx = rg_agent::Ctx {
        version: VERSION.into(),
        ops: Arc::new(AgentOps::new(app.clone())),
        app: app.clone(),
    };
    rg_agent::main(std::env::args_os(), &ctx, &|config| run_gui(&app, config))
}

fn exit(r: anyhow::Result<()>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The GUI (no command, or `gui`); `config` is `--config`'s VM file.
fn run_gui(app: &AppDir, config: Option<&Path>) -> anyhow::Result<()> {
    let backend = RealBackend::new(app.clone(), config)?;
    rg_ui::run(
        Box::new(backend),
        RunOptions {
            x11_embed: true,
            self_test: false,
        },
    )
}

/// `--self-test` (the package check runs it under Xvfb): the MCP server's initialize and
/// tools/list in-process, then the GUI on the real backend draws a few frames. Both use a
/// temporary config folder and Workspace, so no user data is read or written.
fn self_test() -> anyhow::Result<()> {
    let dir = TempDir::new()?;
    let app = AppDir::at(dir.0.join("config"));
    Registry::open(&app)?.set_workspace_root(&dir.0.join("VMs").to_string_lossy())?;
    let ctx = rg_agent::Ctx {
        version: VERSION.into(),
        ops: Arc::new(AgentOps::new(app.clone())),
        app: app.clone(),
    };
    let tools = rg_agent::self_test(ctx)?;
    println!("self-test: {} MCP tools", tools.len());
    rg_ui::run(
        Box::new(RealBackend::new(app, None)?),
        RunOptions {
            x11_embed: false,
            self_test: true,
        },
    )
}

/// A private folder under the temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> std::io::Result<Self> {
        let p = std::env::temp_dir().join(format!("reims-gui-self-test-{}", std::process::id()));
        std::fs::create_dir(&p)?;
        Ok(Self(p))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

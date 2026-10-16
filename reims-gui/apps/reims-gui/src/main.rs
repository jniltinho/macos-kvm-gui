//! reims-gui: manage macOS VMs on QEMU/KVM with Reims vGPU. One binary: the GUI, the CLI and
//! the MCP server.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

fn main() -> ExitCode {
    // Integration hook: rg-ops' `Ops` replaces `Unwired` once that crate exists.
    let ops: Arc<dyn rg_agent::Ops> = Arc::new(rg_agent::Unwired);
    let ctx = match rg_agent::Ctx::user(env!("CARGO_PKG_VERSION"), ops) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    rg_agent::main(std::env::args_os(), &ctx, &run_gui)
}

/// The GUI entry point (no command, or `gui`); `config` is `--config`'s VM file.
// Integration hook: call `rg_ui::run(ctx, config)` once rg-ui provides it.
fn run_gui(_config: Option<&Path>) -> anyhow::Result<()> {
    anyhow::bail!("the desktop app is not built into this binary yet: use a command (see --help)")
}

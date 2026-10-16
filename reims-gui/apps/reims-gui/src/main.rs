//! reims-gui: manage macOS VMs on QEMU/KVM with Reims vGPU. One binary: the GUI, the CLI and
//! the MCP server.

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--version") => println!("reims-gui {}", env!("CARGO_PKG_VERSION")),
        // `--self-test`: the package check (scripts/check-package.sh) runs it under Xvfb.
        // For now it only proves the binary loads and runs; once rg-ui is wired in it SHALL open
        // the GUI, render one frame (lavapipe under Xvfb) and exit 0, non-zero on any failure.
        Some("--self-test") => println!("reims-gui {} self-test ok", env!("CARGO_PKG_VERSION")),
        _ => {}
    }
}

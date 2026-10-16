//! reims-gui: manage macOS VMs on QEMU/KVM with Reims vGPU. One binary: the GUI, the CLI and
//! the MCP server.

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("reims-gui {}", env!("CARGO_PKG_VERSION"));
    }
}

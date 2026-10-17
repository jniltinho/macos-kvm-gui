//! `cargo run -p rg-ui --example demo`: the window on sample data (what `reims-gui --demo` runs).

fn main() -> anyhow::Result<()> {
    rg_ui::run_demo()
}

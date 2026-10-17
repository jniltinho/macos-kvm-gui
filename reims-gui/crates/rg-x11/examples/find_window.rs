//! Diagnostic: `PARENT=<xid> find_window <pid> <reims|qemu> [secs] [embed]` finds a running
//! QEMU's window, optionally embeds it into PARENT and reports what happens for 10 s.

use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let pid: u32 = args.next().ok_or("pid")?.parse()?;
    let kind = match args.next().as_deref() {
        Some("reims") => rg_x11::WindowKind::Reims,
        _ => rg_x11::WindowKind::Qemu,
    };
    let secs: u64 = args.next().map_or(Ok(10), |s| s.parse())?;
    let do_embed = args.next().as_deref() == Some("embed");
    let parent: u32 = std::env::var("PARENT")?.parse()?;
    let e = rg_x11::Embedder::new(None, parent)?;
    let t = Instant::now();
    let w = e.find_window(pid, kind, Duration::from_secs(secs))?;
    println!("found {w:#x} after {:?}", t.elapsed());
    if do_embed {
        let mut emb = e.embed(w)?;
        emb.set_rect(400, 160, 900, 620)?;
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(500));
            emb.pump()?;
            println!("alive={}", emb.is_alive());
        }
        std::mem::forget(emb); // keep it embedded after exit for inspection
    }
    Ok(())
}

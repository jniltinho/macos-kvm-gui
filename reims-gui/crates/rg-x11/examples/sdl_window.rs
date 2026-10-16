//! Test helper: a real SDL3 window titled by argv[1] (default "Reims vGPU"), like Reims vGPU's.

use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let title = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Reims vGPU".into());
    let sdl = sdl3::init()?;
    let _window = sdl.video()?.window(&title, 320, 240).build()?;
    let mut events = sdl.event_pump()?;
    loop {
        for e in events.poll_iter() {
            if let sdl3::event::Event::Quit { .. } = e {
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

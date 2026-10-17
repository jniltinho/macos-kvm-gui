//! A minimal RFB 3.8 (VNC, RFC 6143) client for QEMU's private Unix socket (Go `vnc.go`): no
//! authentication, 32-bit true color, Raw and CopyRect, DesktopSize, and QEMU's extended key
//! events so keys travel as physical scancodes and the guest's own keyboard layout applies.
//!
//! [`Vnc::dial`] returns a [`Vnc`] handle (input, frame copies; cheap to clone and share) and a
//! [`VncReader`] whose [`VncReader::run`] reads updates until the connection closes.

use crate::keys::KeyCode;
use crate::qmp::{TIMEOUT, dial_unix};
use crate::{Error, Result};
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

/// The largest screen side accepted from the server.
const MAX_FRAME: u32 = 8192;
const ENC_RAW: i32 = 0;
const ENC_COPY_RECT: i32 = 1;
const ENC_DESKTOP_SIZE: i32 = -223;
const ENC_QEMU_EXT_KEY: i32 = -258;
/// The pause between the events of a chord: without one macOS misses the modifiers of a
/// shortcut such as Shift+Command+T.
pub const CHORD_PAUSE: Duration = Duration::from_millis(30);

/// Pointer button bits.
pub const BUTTON_LEFT: u8 = 1;
pub const BUTTON_MIDDLE: u8 = 2;
pub const BUTTON_RIGHT: u8 = 4;
pub const BUTTON_WHEEL_UP: u8 = 8;
pub const BUTTON_WHEEL_DOWN: u8 = 16;

/// The guest's screen: RGBA, alpha always 255.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Frame {
    fn blank(width: u32, height: u32) -> Self {
        let mut pixels = vec![0; width as usize * height as usize * 4];
        pixels.iter_mut().skip(3).step_by(4).for_each(|a| *a = 255);
        Self {
            width,
            height,
            pixels,
        }
    }

    fn offset(&self, x: u32, y: u32) -> usize {
        (y as usize * self.width as usize + x as usize) * 4
    }
}

/// A rectangle of the framebuffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// One framebuffer update, for the reader's callback.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Update {
    /// The rectangles that changed (for partial texture uploads).
    pub dirty: Vec<Rect>,
    /// The screen size changed: the whole frame is new (and blank until the next update).
    pub resized: bool,
}

struct Shared {
    writer: tokio::sync::Mutex<OwnedWriteHalf>,
    frame: Mutex<Frame>,
    ext_keys: AtomicBool,
    /// Pixel rectangles received since the last resize ([`Vnc::snapshot`] waits for them).
    pixels: AtomicU64,
    /// Bumped after each framebuffer update: the latest-frame signal for any number of views.
    generation: tokio::sync::watch::Sender<u64>,
}

/// A connection to QEMU's display: input and frame copies, from any task.
#[derive(Clone)]
pub struct Vnc(Arc<Shared>);

/// The reading half of a connection; [`VncReader::run`] consumes it.
pub struct VncReader {
    r: BufReader<OwnedReadHalf>,
    shared: Arc<Shared>,
    pace: Option<Box<dyn Fn(u32, u32) -> Duration + Send>>,
    /// Raw rectangles' wire bytes, reused between updates.
    buf: Vec<u8>,
}

impl Vnc {
    /// Waits for QEMU to create the socket and completes the RFB handshake.
    pub async fn dial(path: &Path) -> Result<(Vnc, VncReader)> {
        let s = dial_unix(path)
            .await
            .map_err(|e| Error::msg(format!("VNC unavailable: {e}")))?;
        Self::handshake(s).await
    }

    /// Runs the RFB handshake on a connected stream.
    pub async fn handshake(s: UnixStream) -> Result<(Vnc, VncReader)> {
        tokio::time::timeout(TIMEOUT, Self::handshake_inner(s))
            .await
            .map_err(|_| Error::msg("VNC: handshake timed out"))?
    }

    async fn handshake_inner(s: UnixStream) -> Result<(Vnc, VncReader)> {
        let (r, mut w) = s.into_split();
        let mut r = BufReader::with_capacity(1 << 20, r);
        let mut ver = [0u8; 12];
        r.read_exact(&mut ver)
            .await
            .map_err(|e| Error::msg(format!("VNC: no protocol version: {e}")))?;
        if &ver[..4] != b"RFB " {
            return Err(Error::msg(format!(
                "VNC: not an RFB server ({:?})",
                String::from_utf8_lossy(&ver)
            )));
        }
        w.write_all(b"RFB 003.008\n").await?;
        let n = r.read_u8().await?;
        if n == 0 {
            return Err(Error::msg(format!(
                "VNC refused the connection: {}",
                read_reason(&mut r).await
            )));
        }
        let mut types = vec![0u8; usize::from(n)];
        r.read_exact(&mut types).await?;
        if !types.contains(&1) {
            return Err(Error::msg(
                "VNC requires authentication; only a private socket without a password is supported",
            ));
        }
        w.write_all(&[1]).await?; // security type None
        if r.read_u32().await? != 0 {
            return Err(Error::msg(format!(
                "VNC security failed: {}",
                read_reason(&mut r).await
            )));
        }
        w.write_all(&[1]).await?; // ClientInit: shared
        let (width, height) = (
            u32::from(r.read_u16().await?),
            u32::from(r.read_u16().await?),
        );
        let mut format = [0u8; 16];
        r.read_exact(&mut format).await?;
        let name_len = r.read_u32().await?;
        tokio::io::copy(
            &mut (&mut r).take(u64::from(name_len)),
            &mut tokio::io::sink(),
        )
        .await?;
        check_size(width, height)?;

        // SetPixelFormat: 32 bpp, depth 24, little endian, true color, R/G/B at bits 16/8/0: the
        // wire bytes are B, G, R, x, QEMU's own surface format (x8r8g8b8), so the server copies
        // instead of converting and the client swaps to RGBA (`bgrx_to_rgba`).
        let mut init = vec![
            0, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0,
        ];
        let encs = [ENC_COPY_RECT, ENC_RAW, ENC_DESKTOP_SIZE, ENC_QEMU_EXT_KEY];
        init.extend([2, 0, 0, encs.len() as u8]);
        for e in encs {
            init.extend(e.to_be_bytes());
        }
        init.extend(update_request(false, width, height));
        w.write_all(&init).await?;

        let shared = Arc::new(Shared {
            writer: tokio::sync::Mutex::new(w),
            frame: Mutex::new(Frame::blank(width, height)),
            ext_keys: AtomicBool::new(false),
            pixels: AtomicU64::new(0),
            generation: tokio::sync::watch::channel(0).0,
        });
        Ok((
            Vnc(Arc::clone(&shared)),
            VncReader {
                r,
                shared,
                pace: None,
                buf: Vec::new(),
            },
        ))
    }

    async fn write(&self, b: &[u8]) -> Result<()> {
        let mut w = self.0.writer.lock().await;
        tokio::time::timeout(TIMEOUT, w.write_all(b))
            .await
            .map_err(|_| Error::msg("VNC: write timed out"))??;
        Ok(())
    }

    /// The current framebuffer size.
    pub fn size(&self) -> (u32, u32) {
        let f = self.0.frame.lock();
        (f.width, f.height)
    }

    /// Copies the framebuffer into `dst`, reallocating it when the size changed.
    pub fn frame_into(&self, dst: &mut Frame) {
        let f = self.0.frame.lock();
        dst.clone_from(&f);
    }

    /// A copy of the framebuffer.
    pub fn frame(&self) -> Frame {
        self.0.frame.lock().clone()
    }

    /// Runs `f` on the framebuffer without copying it (for partial texture uploads).
    pub fn with_frame<T>(&self, f: impl FnOnce(&Frame) -> T) -> T {
        f(&self.0.frame.lock())
    }

    /// A receiver that changes after every framebuffer update (the frame's generation): a view
    /// waits on it and copies only the latest frame, never a queue of them.
    pub fn updates(&self) -> tokio::sync::watch::Receiver<u64> {
        self.0.generation.subscribe()
    }

    /// Whether the server takes QEMU extended key events (it says so in its first update).
    pub fn extended_keys(&self) -> bool {
        self.0.ext_keys.load(Ordering::Relaxed)
    }

    /// A key press or release. With QEMU extended key events the scancode goes too, so the
    /// guest applies its own layout; otherwise only the keysym is sent.
    pub async fn key(&self, down: bool, k: KeyCode) -> Result<()> {
        if self.extended_keys() && k.scancode != 0 {
            let mut b = vec![255, 0, 0, u8::from(down)];
            b.extend(k.keysym.to_be_bytes());
            b.extend(k.scancode.to_be_bytes());
            return self.write(&b).await;
        }
        let mut b = vec![4, u8::from(down), 0, 0];
        b.extend(k.keysym.to_be_bytes());
        self.write(&b).await
    }

    /// Presses keys in order and releases them in reverse, as a person would.
    pub async fn chord(&self, keys: &[KeyCode]) -> Result<()> {
        for k in keys {
            self.key(true, *k).await?;
            tokio::time::sleep(CHORD_PAUSE).await;
        }
        for k in keys.iter().rev() {
            self.key(false, *k).await?;
            tokio::time::sleep(CHORD_PAUSE).await;
        }
        Ok(())
    }

    /// The pointer position (framebuffer coordinates) and pressed buttons.
    pub async fn pointer(&self, buttons: u8, x: u32, y: u32) -> Result<()> {
        let clamp = |v: u32| u16::try_from(v).unwrap_or(u16::MAX).to_be_bytes();
        let mut b = vec![5, buttons];
        b.extend(clamp(x));
        b.extend(clamp(y));
        self.write(&b).await
    }

    /// Waits on a running connection for a complete frame: pixel data since the last resize,
    /// then until updates stop for 300 ms. `updates` gets a value per update (send one from
    /// the reader's callback) and `run_done` resolves with the reader's result, so a dropped
    /// connection is an error, not a blank image.
    pub async fn snapshot(
        &self,
        updates: &mut tokio::sync::mpsc::Receiver<()>,
        run_done: &mut (impl Future<Output = Result<()>> + Unpin),
        timeout: Duration,
    ) -> Result<Frame> {
        const QUIET: Duration = Duration::from_millis(300);
        let deadline = tokio::time::Instant::now() + timeout;
        let got = || self.0.pixels.load(Ordering::Relaxed) > 0;
        let mut quiet_at = got().then(|| tokio::time::Instant::now() + QUIET);
        loop {
            let quiet = async move {
                match quiet_at {
                    Some(t) => tokio::time::sleep_until(t).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                u = updates.recv() => {
                    if u.is_some() && got() {
                        quiet_at = Some(tokio::time::Instant::now() + QUIET);
                    }
                }
                () = quiet => return Ok(self.frame()),
                r = &mut *run_done => {
                    let why = r.err().map_or_else(|| "closed".to_string(), |e| e.to_string());
                    return Err(Error::msg(format!("the VM screen connection closed: {why}")));
                }
                () = tokio::time::sleep_until(deadline) => {
                    return if got() { Ok(self.frame()) } else { Err(Error::msg("no screen image from the VM in time")) };
                }
            }
        }
    }

    /// Dials, waits for a complete frame (see [`Vnc::snapshot`]) and closes.
    pub async fn capture(path: &Path, timeout: Duration) -> Result<Frame> {
        let (v, reader) = Vnc::dial(path).await?;
        let (tx, mut updates) = tokio::sync::mpsc::channel(64);
        let mut run = Box::pin(reader.run(move |_| {
            let _ = tx.try_send(());
        }));
        v.snapshot(&mut updates, &mut run, timeout).await
    }
}

impl VncReader {
    /// The least time between update requests, from the screen size: each request makes QEMU
    /// read the guest's frame, which costs Reims vGPU a GPU readback.
    pub fn with_pace(mut self, pace: impl Fn(u32, u32) -> Duration + Send + 'static) -> Self {
        self.pace = Some(Box::new(pace));
        self
    }

    /// Reads server messages until the connection closes, calling `on_update` after each
    /// framebuffer update.
    pub async fn run(mut self, mut on_update: impl FnMut(&Update) + Send) -> Result<()> {
        let mut last: Option<Instant> = None;
        loop {
            match self.r.read_u8().await? {
                0 => {
                    let u = self.read_update().await?;
                    self.shared.generation.send_modify(|g| *g += 1);
                    on_update(&u);
                    // After a resize the new frame is blank, so ask for all of it.
                    if let Some(pace) = &self.pace {
                        let (w, h) = self.size();
                        let wait =
                            pace(w, h).saturating_sub(last.map_or(Duration::MAX, |l| l.elapsed()));
                        if !wait.is_zero() {
                            tokio::time::sleep(wait).await;
                        }
                        last = Some(Instant::now());
                    }
                    let (w, h) = self.size();
                    let req = update_request(!u.resized, w, h);
                    Vnc(Arc::clone(&self.shared)).write(&req).await?;
                }
                1 => {
                    // SetColourMapEntries: unused with true color.
                    let mut h = [0u8; 5];
                    self.r.read_exact(&mut h).await?;
                    self.skip(u64::from(u16::from_be_bytes([h[3], h[4]])) * 6)
                        .await?;
                }
                2 => {} // Bell
                3 => {
                    // ServerCutText: clipboard sharing is out of scope.
                    let mut pad = [0u8; 3];
                    self.r.read_exact(&mut pad).await?;
                    let n = self.r.read_u32().await?;
                    self.skip(u64::from(n)).await?;
                }
                t => return Err(Error::msg(format!("VNC: unsupported server message {t}"))),
            }
        }
    }

    fn size(&self) -> (u32, u32) {
        let f = self.shared.frame.lock();
        (f.width, f.height)
    }

    async fn skip(&mut self, n: u64) -> Result<()> {
        tokio::io::copy(&mut (&mut self.r).take(n), &mut tokio::io::sink()).await?;
        Ok(())
    }

    async fn read_update(&mut self) -> Result<Update> {
        let _pad = self.r.read_u8().await?;
        let rects = self.r.read_u16().await?;
        let mut u = Update::default();
        for _ in 0..rects {
            let (x, y) = (
                u32::from(self.r.read_u16().await?),
                u32::from(self.r.read_u16().await?),
            );
            let (w, h) = (
                u32::from(self.r.read_u16().await?),
                u32::from(self.r.read_u16().await?),
            );
            let rect = Rect { x, y, w, h };
            match self.r.read_i32().await? {
                ENC_RAW => {
                    self.read_raw(rect).await?;
                    self.shared.pixels.fetch_add(1, Ordering::Relaxed);
                    u.dirty.push(rect);
                }
                ENC_COPY_RECT => {
                    let (sx, sy) = (
                        u32::from(self.r.read_u16().await?),
                        u32::from(self.r.read_u16().await?),
                    );
                    copy_rect(&mut self.shared.frame.lock(), sx, sy, rect);
                    self.shared.pixels.fetch_add(1, Ordering::Relaxed);
                    u.dirty.push(rect);
                }
                ENC_DESKTOP_SIZE => {
                    check_size(w, h)?;
                    *self.shared.frame.lock() = Frame::blank(w, h);
                    self.shared.pixels.store(0, Ordering::Relaxed);
                    u.resized = true;
                    u.dirty.clear();
                }
                ENC_QEMU_EXT_KEY => self.shared.ext_keys.store(true, Ordering::Relaxed),
                e => return Err(Error::msg(format!("VNC: unsupported encoding {e}"))),
            }
        }
        Ok(u)
    }

    /// Reads a Raw rectangle into the reused buffer, then converts it into the frame row by
    /// row under one lock.
    async fn read_raw(&mut self, r: Rect) -> Result<()> {
        let row = r.w as usize * 4;
        self.buf.resize(row * r.h as usize, 0);
        self.r.read_exact(&mut self.buf).await?;
        let mut f = self.shared.frame.lock();
        if r.x >= f.width {
            return Ok(());
        }
        let n = r.w.min(f.width - r.x) as usize * 4;
        for (j, src) in (r.y..f.height).zip(self.buf.chunks_exact(row)) {
            let off = f.offset(r.x, j);
            bgrx_to_rgba(&src[..n], &mut f.pixels[off..off + n]);
        }
        Ok(())
    }
}

/// B, G, R, x pixels to R, G, B, 255; a plain loop the compiler vectorizes.
pub fn bgrx_to_rgba(src: &[u8], dst: &mut [u8]) {
    for (d, s) in dst
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(src.as_chunks::<4>().0)
    {
        *d = [s[2], s[1], s[0], 255];
    }
}

fn check_size(w: u32, h: u32) -> Result<()> {
    if w == 0 || h == 0 || w > MAX_FRAME || h > MAX_FRAME {
        return Err(Error::msg(format!("VNC: implausible screen size {w}x{h}")));
    }
    Ok(())
}

fn update_request(incremental: bool, w: u32, h: u32) -> [u8; 10] {
    let [w0, w1] = u16::try_from(w).unwrap_or(u16::MAX).to_be_bytes();
    let [h0, h1] = u16::try_from(h).unwrap_or(u16::MAX).to_be_bytes();
    [3, u8::from(incremental), 0, 0, 0, 0, w0, w1, h0, h1]
}

/// CopyRect through a copy, since source and destination may overlap.
fn copy_rect(f: &mut Frame, sx: u32, sy: u32, d: Rect) {
    if sx + d.w > f.width || d.x + d.w > f.width || sy + d.h > f.height || d.y + d.h > f.height {
        return;
    }
    let row = d.w as usize * 4;
    let mut tmp = Vec::with_capacity(row * d.h as usize);
    for j in 0..d.h {
        let o = f.offset(sx, sy + j);
        tmp.extend_from_slice(&f.pixels[o..o + row]);
    }
    for (j, src) in (0..d.h).zip(tmp.chunks_exact(row)) {
        let o = f.offset(d.x, d.y + j);
        f.pixels[o..o + row].copy_from_slice(src);
    }
}

async fn read_reason(r: &mut BufReader<OwnedReadHalf>) -> String {
    let Ok(n) = r.read_u32().await else {
        return "unknown reason".into();
    };
    if n > 1 << 16 {
        return "unknown reason".into();
    }
    let mut b = vec![0u8; n as usize];
    match r.read_exact(&mut b).await {
        Ok(_) => String::from_utf8_lossy(&b).into_owned(),
        Err(_) => "unknown reason".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::key_for;
    use std::path::PathBuf;
    use tokio::net::UnixListener;
    use tokio::sync::mpsc;

    /// Accepts one client, runs the server side of the handshake and hands over the stream.
    fn fake_rfb(
        dir: &Path,
        w: u16,
        h: u16,
        security: &'static [u8],
    ) -> std::io::Result<(PathBuf, mpsc::Receiver<UnixStream>)> {
        let sock = dir.join("vnc.sock");
        let l = UnixListener::bind(&sock)?;
        let (tx, rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let Ok((mut c, _)) = l.accept().await else {
                return;
            };
            let mut buf = [0u8; 12];
            let _ = c.write_all(b"RFB 003.008\n").await;
            let _ = c.read_exact(&mut buf).await;
            let mut sec = vec![security.len() as u8];
            sec.extend(security);
            let _ = c.write_all(&sec).await;
            if !security.contains(&1) {
                return;
            }
            let _ = c.read_exact(&mut buf[..1]).await;
            let _ = c.write_all(&[0, 0, 0, 0]).await;
            let _ = c.read_exact(&mut buf[..1]).await;
            let mut si = Vec::new();
            si.extend(w.to_be_bytes());
            si.extend(h.to_be_bytes());
            si.extend([0u8; 16]);
            si.extend(4u32.to_be_bytes());
            si.extend(b"QEMU");
            let _ = c.write_all(&si).await;
            let _ = tx.send(c).await;
        });
        Ok((sock, rx))
    }

    fn rect(x: u16, y: u16, w: u16, h: u16, enc: i32) -> Vec<u8> {
        [
            x.to_be_bytes(),
            y.to_be_bytes(),
            w.to_be_bytes(),
            h.to_be_bytes(),
        ]
        .concat()
        .into_iter()
        .chain(enc.to_be_bytes())
        .collect()
    }

    async fn read_n(s: &mut UnixStream, n: usize) -> std::io::Result<Vec<u8>> {
        let mut b = vec![0; n];
        s.read_exact(&mut b).await?;
        Ok(b)
    }

    #[tokio::test]
    async fn handshake_updates_resize_and_input() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (sock, mut rx) = fake_rfb(dir.path(), 4, 2, &[1])?;
        let (v, reader) = Vnc::dial(&sock).await?;
        let mut srv = rx.recv().await.ok_or_else(|| Error::msg("no server"))?;

        // SetPixelFormat (20) + SetEncodings (4 + 4*4) + full FramebufferUpdateRequest (10).
        let init = read_n(&mut srv, 50).await?;
        assert_eq!(
            (init[0], init[4], init[7], init[14], init[15], init[16]),
            (0, 32, 1, 16, 8, 0)
        );
        assert_eq!(
            i32::from_be_bytes([init[36], init[37], init[38], init[39]]),
            ENC_QEMU_EXT_KEY
        );
        assert_eq!(
            (init[40], init[41]),
            (3, 0),
            "first request must be a full update"
        );

        let (tx, mut updates) = mpsc::channel(4);
        tokio::spawn(reader.run(move |u| {
            let _ = tx.try_send(u.clone());
        }));

        // One update: ext-key ack, a raw 4x2 frame, then CopyRect of pixel (0,0) onto (3,1).
        let mut u = vec![0, 0, 0, 3];
        u.extend(rect(0, 0, 0, 0, ENC_QEMU_EXT_KEY));
        u.extend(rect(0, 0, 4, 2, ENC_RAW));
        for i in 0..8u8 {
            u.extend([2, 1, 10 * i, 0]); // B, G, R, x: x arrives as 0
        }
        u.extend(rect(3, 1, 1, 1, ENC_COPY_RECT));
        u.extend([0, 0, 0, 0]);
        srv.write_all(&u).await?;
        let mut generation = v.updates();
        let up = updates.recv().await.unwrap_or_default();
        assert_eq!(up.dirty.len(), 2);
        assert!(
            generation.has_changed().unwrap_or(false),
            "the watch channel saw the update"
        );
        generation.mark_unchanged();
        let f = v.frame();
        assert_eq!((f.pixels[4], f.pixels[3]), (10, 255), "raw pixels");
        assert_eq!(f.pixels[f.offset(3, 1)], 0, "CopyRect");
        let req = read_n(&mut srv, 10).await?;
        assert_eq!((req[0], req[1]), (3, 1), "then an incremental request");

        // Guest resolution change.
        let mut d = vec![0, 0, 0, 1];
        d.extend(rect(0, 0, 8, 6, ENC_DESKTOP_SIZE));
        srv.write_all(&d).await?;
        assert!(updates.recv().await.is_some_and(|u| u.resized));
        assert_eq!(v.size(), (8, 6));
        let req = read_n(&mut srv, 10).await?;
        assert_eq!(
            (req[1], req[7], req[9]),
            (0, 8, 6),
            "a full request of the new size"
        );

        // Input: extended key event (scancode) and pointer.
        v.key(true, key_for("A").unwrap_or_default()).await?;
        let ke = read_n(&mut srv, 12).await?;
        assert_eq!(ke, [255, 0, 0, 1, 0, 0, 0, b'a', 0, 0, 0, 0x1e]);
        v.pointer(BUTTON_LEFT, 5, 4).await?;
        assert_eq!(read_n(&mut srv, 6).await?, [5, 1, 0, 5, 0, 4]);
        Ok(())
    }

    #[tokio::test]
    async fn plain_key_events_without_ext_keys_and_no_auth() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (sock, mut rx) = fake_rfb(dir.path(), 2, 2, &[1])?;
        let (v, _reader) = Vnc::dial(&sock).await?;
        let mut srv = rx.recv().await.ok_or_else(|| Error::msg("no server"))?;
        read_n(&mut srv, 50).await?;
        v.key(false, key_for("Return").unwrap_or_default()).await?;
        assert_eq!(read_n(&mut srv, 8).await?, [4, 0, 0, 0, 0, 0, 0xff, 0x0d]);

        let dir2 = tempfile::tempdir()?;
        let (sock2, _rx2) = fake_rfb(dir2.path(), 2, 2, &[2])?; // VNC password only
        assert!(
            Vnc::dial(&sock2).await.is_err(),
            "authentication must be refused"
        );
        Ok(())
    }

    #[test]
    fn bgrx_to_rgba_swaps_and_is_fast_at_4k() {
        let src: Vec<u8> = (0..3840 * 2160 * 4).map(|i| (i % 251) as u8).collect();
        let mut dst = vec![0u8; src.len()];
        let t = Instant::now();
        bgrx_to_rgba(&src, &mut dst);
        let took = t.elapsed();
        assert_eq!(&dst[..4], &[src[2], src[1], src[0], 255]);
        // A 4K frame must convert well inside one 60 Hz frame (release); debug builds are slow.
        let limit = if cfg!(debug_assertions) {
            Duration::from_secs(2)
        } else {
            Duration::from_millis(20)
        };
        assert!(took < limit, "4K BGRX→RGBA took {took:?}");
    }

    #[tokio::test]
    async fn capture_waits_for_pixels_then_quiet() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (sock, mut rx) = fake_rfb(dir.path(), 1, 1, &[1])?;
        let server = tokio::spawn(async move {
            let mut srv = rx.recv().await?;
            read_n(&mut srv, 50).await.ok()?;
            let mut u = vec![0, 0, 0, 1];
            u.extend(rect(0, 0, 1, 1, ENC_RAW));
            u.extend([9, 8, 7, 0]);
            srv.write_all(&u).await.ok()?;
            tokio::time::sleep(Duration::from_secs(2)).await; // keep the connection open
            Some(())
        });
        let f = Vnc::capture(&sock, Duration::from_secs(5)).await?;
        assert_eq!(f.pixels, [7, 8, 9, 255]);
        server.abort();
        Ok(())
    }
}

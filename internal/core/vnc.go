package core

import (
	"bufio"
	"encoding/binary"
	"errors"
	"fmt"
	"image"
	"io"
	"net"
	"sync"
	"sync/atomic"
	"time"
)

// Minimal RFB 3.8 (VNC, RFC 6143) client for QEMU's private Unix socket: no authentication,
// 32-bit true color, Raw and CopyRect, DesktopSize, and QEMU's extended key events so keys
// travel as physical scancodes and the guest's own keyboard layout applies.

// maxFrame bounds the screen size accepted from the server.
const maxFrame = 8192

const (
	encRaw         = 0
	encCopyRect    = 1
	encDesktopSize = -223
	encQEMUExtKey  = -258
)

// VNC is a connection to QEMU's display. Frame and the input methods are safe to call from
// any goroutine while Run reads updates.
type VNC struct {
	conn net.Conn
	r    *bufio.Reader
	wmu  sync.Mutex // serializes writes

	mu  sync.Mutex // guards img
	img *image.RGBA

	extKeys  atomic.Bool
	pixels   atomic.Int64 // pixel rectangles received since the last resize (Snapshot waits for them)
	resized  bool         // set by DesktopSize: the next request must be a full one; read loop only
	OnUpdate func()       // called after each framebuffer update; may be nil
	// Pace, when set before Run, is the least time between update requests: each one makes
	// QEMU read the guest's frame, which costs Reims vGPU a GPU readback.
	Pace func() time.Duration
}

// DialVNC waits for QEMU to create the socket and completes the RFB handshake.
func DialVNC(path string) (*VNC, error) {
	var conn net.Conn
	var err error
	for range qmpDialTries {
		if conn, err = net.Dial("unix", path); err == nil {
			break
		}
		time.Sleep(qmpDialBackoff)
	}
	if err != nil {
		return nil, fmt.Errorf("VNC unavailable: %w", err)
	}
	v, err := NewVNC(conn)
	if err != nil {
		conn.Close()
	}
	return v, err
}

// NewVNC runs the RFB handshake on conn.
func NewVNC(conn net.Conn) (*VNC, error) {
	v := &VNC{conn: conn, r: bufio.NewReaderSize(conn, 1<<20)}
	conn.SetDeadline(time.Now().Add(qmpTimeout))
	defer conn.SetDeadline(time.Time{})

	var ver [12]byte
	if _, err := io.ReadFull(v.r, ver[:]); err != nil {
		return nil, fmt.Errorf("VNC: no protocol version: %w", err)
	}
	if string(ver[:4]) != "RFB " {
		return nil, fmt.Errorf("VNC: not an RFB server (%q)", ver)
	}
	if _, err := conn.Write([]byte("RFB 003.008\n")); err != nil {
		return nil, err
	}
	var n uint8
	if err := binary.Read(v.r, binary.BigEndian, &n); err != nil {
		return nil, err
	}
	if n == 0 {
		return nil, fmt.Errorf("VNC refused the connection: %s", v.readString())
	}
	types := make([]byte, n)
	if _, err := io.ReadFull(v.r, types); err != nil {
		return nil, err
	}
	if !containsByte(types, 1) {
		return nil, errors.New("VNC requires authentication; only a private socket without a password is supported")
	}
	if _, err := conn.Write([]byte{1}); err != nil { // security type None
		return nil, err
	}
	var result uint32
	if err := binary.Read(v.r, binary.BigEndian, &result); err != nil {
		return nil, err
	}
	if result != 0 {
		return nil, fmt.Errorf("VNC security failed: %s", v.readString())
	}
	if _, err := conn.Write([]byte{1}); err != nil { // ClientInit: shared
		return nil, err
	}
	var init struct {
		W, H   uint16
		Format [16]byte
		Name   uint32
	}
	if err := binary.Read(v.r, binary.BigEndian, &init); err != nil {
		return nil, fmt.Errorf("VNC: bad ServerInit: %w", err)
	}
	if _, err := io.CopyN(io.Discard, v.r, int64(init.Name)); err != nil {
		return nil, err
	}
	if init.W == 0 || init.H == 0 || init.W > maxFrame || init.H > maxFrame {
		return nil, fmt.Errorf("VNC: implausible screen size %dx%d", init.W, init.H)
	}
	v.img = blankFrame(int(init.W), int(init.H))

	// SetPixelFormat: 32 bpp, depth 24, little endian, true color, R/G/B at bits 0/8/16,
	// so the wire bytes are R, G, B, x, the same order as image.RGBA.
	pf := []byte{0, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 0, 8, 16, 0, 0, 0}
	encs := []int32{encCopyRect, encRaw, encDesktopSize, encQEMUExtKey}
	se := make([]byte, 4, 4+4*len(encs))
	se[0] = 2
	binary.BigEndian.PutUint16(se[2:], uint16(len(encs)))
	for _, e := range encs {
		se = binary.BigEndian.AppendUint32(se, uint32(e))
	}
	if err := v.write(append(pf, se...)); err != nil {
		return nil, err
	}
	return v, v.requestUpdate(false)
}

func containsByte(b []byte, c byte) bool {
	for _, x := range b {
		if x == c {
			return true
		}
	}
	return false
}

func (v *VNC) readString() string {
	var n uint32
	if binary.Read(v.r, binary.BigEndian, &n) != nil || n > 1<<16 {
		return "unknown reason"
	}
	b := make([]byte, n)
	io.ReadFull(v.r, b)
	return string(b)
}

func blankFrame(w, h int) *image.RGBA {
	img := image.NewRGBA(image.Rect(0, 0, w, h))
	for i := 3; i < len(img.Pix); i += 4 {
		img.Pix[i] = 255
	}
	return img
}

func (v *VNC) write(b []byte) error {
	v.wmu.Lock()
	defer v.wmu.Unlock()
	v.conn.SetWriteDeadline(time.Now().Add(qmpTimeout))
	_, err := v.conn.Write(b)
	return err
}

func (v *VNC) requestUpdate(incremental bool) error {
	v.mu.Lock()
	w, h := v.img.Bounds().Dx(), v.img.Bounds().Dy()
	v.mu.Unlock()
	b := []byte{3, 0, 0, 0, 0, 0, 0, 0, 0, 0}
	if incremental {
		b[1] = 1
	}
	binary.BigEndian.PutUint16(b[6:], uint16(w))
	binary.BigEndian.PutUint16(b[8:], uint16(h))
	return v.write(b)
}

// Size is the current framebuffer size.
func (v *VNC) Size() (int, int) {
	v.mu.Lock()
	defer v.mu.Unlock()
	return v.img.Bounds().Dx(), v.img.Bounds().Dy()
}

// Frame copies the framebuffer into dst, reallocating it when the size changed.
func (v *VNC) Frame(dst *image.RGBA) *image.RGBA {
	v.mu.Lock()
	defer v.mu.Unlock()
	if dst == nil || dst.Rect != v.img.Rect {
		dst = image.NewRGBA(v.img.Rect)
	}
	copy(dst.Pix, v.img.Pix)
	return dst
}

// Run reads server messages until the connection closes.
func (v *VNC) Run() error {
	var last time.Time
	for {
		t, err := v.r.ReadByte()
		if err != nil {
			return err
		}
		switch t {
		case 0:
			if err := v.readUpdate(); err != nil {
				return err
			}
			if v.OnUpdate != nil {
				v.OnUpdate()
			}
			// After a resize the new frame is blank, so ask for all of it.
			full := v.resized
			v.resized = false
			if v.Pace != nil {
				if d := v.Pace() - time.Since(last); d > 0 {
					time.Sleep(d)
				}
				last = time.Now()
			}
			if err := v.requestUpdate(!full); err != nil {
				return err
			}
		case 1: // SetColourMapEntries: unused with true color
			var h struct {
				Pad         uint8
				First, Size uint16
			}
			if err := binary.Read(v.r, binary.BigEndian, &h); err != nil {
				return err
			}
			if _, err := io.CopyN(io.Discard, v.r, int64(h.Size)*6); err != nil {
				return err
			}
		case 2: // Bell
		case 3: // ServerCutText: clipboard sharing is out of scope
			var h struct {
				Pad [3]byte
				Len uint32
			}
			if err := binary.Read(v.r, binary.BigEndian, &h); err != nil {
				return err
			}
			if _, err := io.CopyN(io.Discard, v.r, int64(h.Len)); err != nil {
				return err
			}
		default:
			return fmt.Errorf("VNC: unsupported server message %d", t)
		}
	}
}

func (v *VNC) readUpdate() error {
	var h struct {
		Pad   uint8
		Rects uint16
	}
	if err := binary.Read(v.r, binary.BigEndian, &h); err != nil {
		return err
	}
	for range h.Rects {
		var r struct {
			X, Y, W, H uint16
			Enc        int32
		}
		if err := binary.Read(v.r, binary.BigEndian, &r); err != nil {
			return err
		}
		x, y, w, hh := int(r.X), int(r.Y), int(r.W), int(r.H)
		switch r.Enc {
		case encRaw:
			if err := v.readRaw(x, y, w, hh); err != nil {
				return err
			}
			v.pixels.Add(1)
		case encCopyRect:
			var src struct{ X, Y uint16 }
			if err := binary.Read(v.r, binary.BigEndian, &src); err != nil {
				return err
			}
			v.copyRect(int(src.X), int(src.Y), x, y, w, hh)
			v.pixels.Add(1)
		case encDesktopSize:
			if w == 0 || hh == 0 || w > maxFrame || hh > maxFrame {
				return fmt.Errorf("VNC: implausible screen size %dx%d", w, hh)
			}
			v.mu.Lock()
			v.img = blankFrame(w, hh)
			v.mu.Unlock()
			v.resized = true
			v.pixels.Store(0)
		case encQEMUExtKey:
			v.extKeys.Store(true) // the server accepts QEMU extended key events
		default:
			return fmt.Errorf("VNC: unsupported encoding %d", r.Enc)
		}
	}
	return nil
}

func (v *VNC) readRaw(x, y, w, h int) error {
	row := make([]byte, w*4)
	for j := range h {
		if _, err := io.ReadFull(v.r, row); err != nil {
			return err
		}
		v.mu.Lock()
		b := v.img.Bounds()
		if y+j < b.Max.Y && x < b.Max.X {
			off := v.img.PixOffset(x, y+j)
			n := min(w, b.Max.X-x) * 4
			copy(v.img.Pix[off:off+n], row[:n])
			for i := off + 3; i < off+n; i += 4 {
				v.img.Pix[i] = 255
			}
		}
		v.mu.Unlock()
	}
	return nil
}

func (v *VNC) copyRect(sx, sy, dx, dy, w, h int) {
	v.mu.Lock()
	defer v.mu.Unlock()
	b := v.img.Bounds()
	if sx+w > b.Max.X || dx+w > b.Max.X || sy+h > b.Max.Y || dy+h > b.Max.Y {
		return
	}
	tmp := make([]byte, w*4*h) // through a copy, because source and destination may overlap
	for j := range h {
		o := v.img.PixOffset(sx, sy+j)
		copy(tmp[j*w*4:], v.img.Pix[o:o+w*4])
	}
	for j := range h {
		o := v.img.PixOffset(dx, dy+j)
		copy(v.img.Pix[o:o+w*4], tmp[j*w*4:(j+1)*w*4])
	}
}

// Key sends a key press or release. With QEMU extended key events the scancode is sent
// too, so the guest applies its own keyboard layout; otherwise only the keysym is sent.
func (v *VNC) Key(down bool, k KeyCode) error {
	if v.extKeys.Load() && k.Scancode != 0 {
		b := make([]byte, 12)
		b[0], b[1] = 255, 0
		if down {
			b[3] = 1
		}
		binary.BigEndian.PutUint32(b[4:], k.Keysym)
		binary.BigEndian.PutUint32(b[8:], k.Scancode)
		return v.write(b)
	}
	b := make([]byte, 8)
	b[0] = 4
	if down {
		b[1] = 1
	}
	binary.BigEndian.PutUint32(b[4:], k.Keysym)
	return v.write(b)
}

// Chord presses keys in order and releases them in reverse, as a person would. Each event waits
// a moment: with none between them macOS misses the modifiers of a shortcut such as Shift+Command+T.
func (v *VNC) Chord(keys []KeyCode) error {
	for _, k := range keys {
		if err := v.Key(true, k); err != nil {
			return err
		}
		time.Sleep(chordPause)
	}
	for i := len(keys) - 1; i >= 0; i-- {
		if err := v.Key(false, keys[i]); err != nil {
			return err
		}
		time.Sleep(chordPause)
	}
	return nil
}

const chordPause = 30 * time.Millisecond

// Pointer button bits.
const (
	ButtonLeft      = 1
	ButtonMiddle    = 2
	ButtonRight     = 4
	ButtonWheelUp   = 8
	ButtonWheelDown = 16
)

// Pointer sends the pointer position (framebuffer coordinates) and pressed buttons.
func (v *VNC) Pointer(buttons uint8, x, y int) error {
	b := make([]byte, 6)
	b[0], b[1] = 5, buttons
	binary.BigEndian.PutUint16(b[2:], uint16(max(x, 0)))
	binary.BigEndian.PutUint16(b[4:], uint16(max(y, 0)))
	return v.write(b)
}

func (v *VNC) Close() error { return v.conn.Close() }

// Snapshot waits on a running connection for a complete frame: pixel data received since the
// last resize (not just QEMU's acknowledgement of an encoding), then until updates stop for a
// moment. updates gets a value per framebuffer update (from OnUpdate, set before Run starts)
// and runErr Run's result, so a dropped connection is an error, not a blank image.
func (v *VNC) Snapshot(updates <-chan struct{}, runErr <-chan error, timeout time.Duration) (*image.RGBA, error) {
	deadline := time.After(timeout)
	quiet := time.NewTimer(time.Hour)
	if v.pixels.Load() > 0 { // a frame arrived already: wait only for the screen to settle
		quiet.Reset(300 * time.Millisecond)
	}
	for {
		select {
		case <-updates:
			if v.pixels.Load() > 0 {
				quiet.Reset(300 * time.Millisecond)
			}
		case <-quiet.C:
			return v.Frame(nil), nil
		case err := <-runErr:
			return nil, fmt.Errorf("the VM screen connection closed: %v", err)
		case <-deadline:
			if v.pixels.Load() > 0 {
				return v.Frame(nil), nil
			}
			return nil, errors.New("no screen image from the VM in time")
		}
	}
}

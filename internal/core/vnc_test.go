package core

import (
	"bytes"
	"encoding/binary"
	"io"
	"net"
	"path/filepath"
	"testing"
	"time"
)

// fakeRFB accepts one client on a Unix socket, runs the server side of the handshake and
// hands the connection to the test.
func fakeRFB(t *testing.T, w, h uint16, security []byte) (string, chan net.Conn) {
	sock := filepath.Join(t.TempDir(), "vnc.sock")
	l, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { l.Close() })
	ch := make(chan net.Conn, 1)
	go func() {
		c, err := l.Accept()
		if err != nil {
			return
		}
		c.Write([]byte("RFB 003.008\n"))
		io.ReadFull(c, make([]byte, 12)) // client version
		c.Write(append([]byte{byte(len(security))}, security...))
		if bytes.IndexByte(security, 1) < 0 {
			c.Close()
			return
		}
		io.ReadFull(c, make([]byte, 1)) // chosen security type
		c.Write([]byte{0, 0, 0, 0})     // SecurityResult OK
		io.ReadFull(c, make([]byte, 1)) // ClientInit
		var si bytes.Buffer
		binary.Write(&si, binary.BigEndian, struct {
			W, H   uint16
			Format [16]byte
			Name   uint32
		}{w, h, [16]byte{}, 4})
		si.WriteString("QEMU")
		c.Write(si.Bytes())
		ch <- c
	}()
	return sock, ch
}

func rect(x, y, w, h uint16, enc int32) []byte {
	b := binary.BigEndian.AppendUint16(nil, x)
	b = binary.BigEndian.AppendUint16(b, y)
	b = binary.BigEndian.AppendUint16(b, w)
	b = binary.BigEndian.AppendUint16(b, h)
	return binary.BigEndian.AppendUint32(b, uint32(enc))
}

func TestVNCHandshakeAndUpdates(t *testing.T) {
	sock, ch := fakeRFB(t, 4, 2, []byte{1})
	v, err := DialVNC(sock)
	if err != nil {
		t.Fatal(err)
	}
	defer v.Close()
	srv := <-ch

	// SetPixelFormat (20) + SetEncodings (4 + 4*4) + full FramebufferUpdateRequest (10).
	init := make([]byte, 20+20+10)
	if _, err := io.ReadFull(srv, init); err != nil {
		t.Fatal(err)
	}
	if init[0] != 0 || init[4] != 32 || init[7] != 1 || init[14] != 0 || init[15] != 8 || init[16] != 16 {
		t.Fatalf("pixel format: % x", init[:20])
	}
	encs := init[24:40]
	if int32(binary.BigEndian.Uint32(encs[12:])) != encQEMUExtKey {
		t.Fatalf("encodings must include QEMU extended keys: % x", encs)
	}
	if init[40] != 3 || init[41] != 0 {
		t.Fatalf("first request must be a full update: % x", init[40:])
	}

	updates := make(chan struct{}, 4)
	v.OnUpdate = func() { updates <- struct{}{} }
	go v.Run()

	// One update: ext-key ack, a raw 4x2 frame, then CopyRect of pixel (0,0) onto (3,1).
	var u bytes.Buffer
	u.Write([]byte{0, 0, 0, 3})
	u.Write(rect(0, 0, 0, 0, encQEMUExtKey))
	u.Write(rect(0, 0, 4, 2, encRaw))
	for i := range 8 {
		u.Write([]byte{byte(10 * i), 1, 2, 0}) // R varies, alpha byte arrives as 0
	}
	u.Write(rect(3, 1, 1, 1, encCopyRect))
	u.Write([]byte{0, 0, 0, 0}) // source (0,0)
	srv.Write(u.Bytes())
	select {
	case <-updates:
	case <-time.After(2 * time.Second):
		t.Fatal("no update")
	}
	f := v.Frame(nil)
	if f.Pix[4*1] != 10 || f.Pix[3] != 255 {
		t.Fatalf("raw pixels: % x", f.Pix[:8])
	}
	if f.Pix[f.PixOffset(3, 1)] != 0 {
		t.Fatalf("CopyRect: got %d at (3,1)", f.Pix[f.PixOffset(3, 1)])
	}
	if req := make([]byte, 10); func() bool { io.ReadFull(srv, req); return req[0] != 3 || req[1] != 1 }() {
		t.Fatal("after an update the client must ask for an incremental one")
	}

	// Guest resolution change.
	var d bytes.Buffer
	d.Write([]byte{0, 0, 0, 1})
	d.Write(rect(0, 0, 8, 6, encDesktopSize))
	srv.Write(d.Bytes())
	<-updates
	if w, h := v.Size(); w != 8 || h != 6 {
		t.Fatalf("DesktopSize: %dx%d", w, h)
	}
	if req := make([]byte, 10); func() bool { io.ReadFull(srv, req); return req[1] != 0 || req[7] != 8 || req[9] != 6 }() {
		t.Fatal("after a resize the client must ask for a full update of the new size")
	}

	// Input: extended key event (scancode) and pointer.
	k, _ := KeyFor("A")
	if err := v.Key(true, k); err != nil {
		t.Fatal(err)
	}
	ke := make([]byte, 12)
	io.ReadFull(srv, ke)
	if ke[0] != 255 || ke[1] != 0 || ke[3] != 1 || binary.BigEndian.Uint32(ke[4:]) != 'a' || binary.BigEndian.Uint32(ke[8:]) != 0x1e {
		t.Fatalf("extended key event: % x", ke)
	}
	v.Pointer(ButtonLeft, 5, 4)
	pe := make([]byte, 6)
	io.ReadFull(srv, pe)
	if !bytes.Equal(pe, []byte{5, 1, 0, 5, 0, 4}) {
		t.Fatalf("pointer event: % x", pe)
	}
}

func TestVNCWithoutExtendedKeysAndAuth(t *testing.T) {
	sock, ch := fakeRFB(t, 2, 2, []byte{1})
	v, err := DialVNC(sock)
	if err != nil {
		t.Fatal(err)
	}
	srv := <-ch
	io.ReadFull(srv, make([]byte, 50))
	k, _ := KeyFor("Return")
	v.Key(false, k) // no ack from the server: plain KeyEvent with the keysym
	ke := make([]byte, 8)
	io.ReadFull(srv, ke)
	if ke[0] != 4 || ke[1] != 0 || binary.BigEndian.Uint32(ke[4:]) != 0xff0d {
		t.Fatalf("key event: % x", ke)
	}
	v.Close()

	sock2, _ := fakeRFB(t, 2, 2, []byte{2}) // VNC password only
	if _, err := DialVNC(sock2); err == nil {
		t.Fatal("a server that requires authentication must be refused")
	}
}

func TestKeyTableAndFit(t *testing.T) {
	for name, want := range map[string]KeyCode{
		"A": {'a', 0x1e}, "Z": {'z', 0x2c}, "1": {'1', 0x02}, "0": {'0', 0x0b},
		"LeftSuper": {0xffeb, 0xdb}, "LeftAlt": {0xffe9, 0x38}, "Up": {0xff52, 0xc8}, "BackSpace": {0xff08, 0x0e},
	} {
		if got, ok := KeyFor(name); !ok || got != want {
			t.Errorf("%s: got %+v want %+v", name, got, want)
		}
	}
	// A 1920x1080 frame in a 960x700 box is drawn at half size with 80 px bars above and below.
	if s, ox, oy := FitRect(1920, 1080, 960, 700); s != 0.5 || ox != 0 || oy != 80 {
		t.Fatalf("fit: %v %v %v", s, ox, oy)
	}
	if x, y, ok := ToFrame(1920, 1080, 960, 700, 480, 350); !ok || x != 960 || y != 540 {
		t.Fatalf("center: %d %d %v", x, y, ok)
	}
	if _, _, ok := ToFrame(1920, 1080, 960, 700, 480, 40); ok {
		t.Fatal("clicks in the letterbox bars must be ignored")
	}
}

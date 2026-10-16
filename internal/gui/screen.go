package gui

import (
	"errors"
	"fmt"
	"image"
	"image/color"
	"runtime"
	"sync"
	"sync/atomic"
	"time"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/canvas"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/driver/desktop"
	"fyne.io/fyne/v2/widget"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

// screen shows the VM's display (over VNC) and sends keyboard and mouse to it while focused.
type screen struct {
	widget.BaseWidget
	img  *canvas.Image
	hint *widget.Label

	mu       sync.Mutex
	vnc      *core.VNC
	buf      *image.RGBA // one image updated in place: Fyne keeps a single texture for it
	buttons  uint8
	lastX    int // last pointer position sent, in frame coordinates
	lastY    int
	lastSent uint8 // buttons sent with the last position
	pressed  map[string]core.KeyCode
	dirty    atomic.Bool
	stop     chan struct{}
	// onHostKey runs when the host key (Ctrl+Alt+F) is pressed; it is never sent to the guest.
	onHostKey func()
	// onFrame receives each drawn frame (on the UI goroutine), for the Details preview.
	onFrame func(*image.RGBA)
}

const (
	screenIdle  = "The VM screen appears here while it runs. Click it to type."
	screenReims = "The screen is in the Reims vGPU window.\n\nThis VM runs on Reims vGPU, which draws macOS in its own window on the host's GPU:\n" +
		"type and click there. The Preview in Details follows it."
)

func newScreen() *screen {
	s := &screen{img: canvas.NewImageFromImage(image.NewRGBA(image.Rect(0, 0, 1, 1))), hint: widget.NewLabel(screenIdle), pressed: map[string]core.KeyCode{}}
	s.img.FillMode = canvas.ImageFillContain
	s.img.ScaleMode = canvas.ImageScaleFastest
	s.hint.Alignment = fyne.TextAlignCenter
	s.ExtendBaseWidget(s)
	return s
}

func (s *screen) CreateRenderer() fyne.WidgetRenderer {
	bg := canvas.NewRectangle(color.Black)
	return widget.NewSimpleRenderer(container.NewStack(bg, s.img, container.NewCenter(s.hint)))
}

func (s *screen) MinSize() fyne.Size { return fyne.NewSize(480, 270) }

// attach starts showing v; frames are redrawn at most 60 times per second.
func (s *screen) attach(v *core.VNC) {
	s.mu.Lock()
	if s.stop != nil {
		close(s.stop) // a connection that was not detached yet
	}
	s.vnc, s.stop = v, make(chan struct{})
	stop := s.stop
	s.mu.Unlock()
	v.OnUpdate = func() { s.dirty.Store(true) }
	s.hint.Hide()
	go func() {
		t := time.NewTicker(time.Second / 60)
		defer t.Stop()
		for {
			select {
			case <-stop:
				return
			case <-t.C:
				if !s.dirty.Swap(false) {
					continue
				}
				fyne.Do(func() {
					s.mu.Lock()
					if s.vnc == v {
						// Runs on Fyne's main goroutine, which also paints, so the copy never races the renderer.
						s.buf = v.Frame(s.buf)
						s.img.Image = s.buf
						if s.onFrame != nil {
							s.onFrame(s.buf)
						}
					}
					s.mu.Unlock()
					s.img.Refresh()
				})
			}
		}
	}()
}

// detach stops showing the VM, releasing anything still pressed.
// detach drops v; a newer connection that replaced it is left alone.
func (s *screen) detach(v *core.VNC) {
	s.mu.Lock()
	if s.vnc != v {
		s.mu.Unlock()
		return
	}
	s.mu.Unlock()
	s.releaseAll()
	s.mu.Lock()
	if s.stop != nil {
		close(s.stop)
	}
	s.vnc, s.stop = nil, nil
	s.mu.Unlock()
	s.img.Image = image.NewRGBA(image.Rect(0, 0, 1, 1))
	s.img.Refresh()
	s.hint.Show()
}

func (s *screen) conn() *core.VNC {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.vnc
}

// Keyboard: physical keys go to the guest as key presses and releases.

func (s *screen) FocusGained()                 {}
func (s *screen) FocusLost()                   { s.releaseAll() }
func (s *screen) TypedRune(rune)               {}
func (s *screen) TypedKey(*fyne.KeyEvent)      {}
func (s *screen) AcceptsTab() bool             { return true } // Tab goes to the guest, not to Fyne's focus chain
func (s *screen) KeyDown(ev *fyne.KeyEvent)    { s.key(ev, true) }
func (s *screen) KeyUp(ev *fyne.KeyEvent)      { s.key(ev, false) }
func (s *screen) Scrolled(e *fyne.ScrollEvent) { s.wheel(e.Scrolled.DY) }

// key sends a physical key. On Linux the X11 keycode identifies the key itself, whatever the
// host's layout, so keys that a US key map lacks (such as ABNT2's ç and "/?") are not lost;
// elsewhere, or for an unknown keycode, the Fyne key name is used.
func (s *screen) key(ev *fyne.KeyEvent, down bool) {
	name := ev.Name
	v := s.conn()
	k, ok := core.KeyFor(string(name))
	if runtime.GOOS == "linux" && ev.Physical.ScanCode > 8 {
		if pk, pok := core.KeyForX11(ev.Physical.ScanCode); pok {
			k, ok = pk, true
		}
	}
	if !ok {
		return
	}
	if name == "" {
		name = fyne.KeyName(fmt.Sprintf("scancode:%d", ev.Physical.ScanCode)) // tracks unnamed keys too
	}
	if down && name == "F" && s.isHeld("LeftControl", "RightControl") && s.isHeld("LeftAlt", "RightAlt") {
		s.releaseAll() // the guest saw Ctrl and Alt go down; let them go up
		if s.onHostKey != nil {
			s.onHostKey()
		}
		return
	}
	s.mu.Lock()
	_, held := s.pressed[string(name)]
	if down == held { // ignore auto-repeat downs and stray ups: macOS repeats keys itself
		s.mu.Unlock()
		return
	}
	if down {
		s.pressed[string(name)] = k
	} else {
		delete(s.pressed, string(name))
	}
	s.mu.Unlock()
	if v != nil { // keys are tracked without a VM too, so the host key still leaves full screen
		v.Key(down, k)
	}
}

func (s *screen) isHeld(names ...string) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, n := range names {
		if _, ok := s.pressed[n]; ok {
			return true
		}
	}
	return false
}

func (s *screen) releaseAll() {
	s.mu.Lock()
	v, keys := s.vnc, s.pressed
	s.pressed = map[string]core.KeyCode{}
	s.mu.Unlock()
	if v == nil {
		return
	}
	for _, k := range keys {
		v.Key(false, k)
	}
}

// Mouse: positions are mapped through the same contain-fit scaling the image uses.

func (s *screen) pointer(pos fyne.Position) {
	v := s.conn()
	if v == nil {
		return
	}
	fw, fh := v.Size()
	size := s.Size()
	x, y, ok := core.ToFrame(fw, fh, size.Width, size.Height, pos.X, pos.Y)
	s.mu.Lock()
	held := s.buttons != 0 || s.lastSent != 0
	s.mu.Unlock()
	if !ok && held && fw > 0 && fh > 0 { // a drag into the black bars: keep it at the frame's edge
		scale, ox, oy := core.FitRect(fw, fh, size.Width, size.Height)
		if scale > 0 {
			x = min(max(int((pos.X-ox)/scale), 0), fw-1)
			y = min(max(int((pos.Y-oy)/scale), 0), fh-1)
			ok = true
		}
	}
	if ok {
		s.mu.Lock()
		b := s.buttons
		same := x == s.lastX && y == s.lastY && b == s.lastSent
		s.lastX, s.lastY, s.lastSent = x, y, b
		s.mu.Unlock()
		if !same { // high-rate motion events often repeat a position after scaling
			v.Pointer(b, x, y)
		}
	}
}

func vncButton(b desktop.MouseButton) uint8 {
	switch b {
	case desktop.MouseButtonPrimary:
		return core.ButtonLeft
	case desktop.MouseButtonSecondary:
		return core.ButtonRight
	case desktop.MouseButtonTertiary:
		return core.ButtonMiddle
	}
	return 0
}

func (s *screen) MouseDown(e *desktop.MouseEvent) {
	if c := fyne.CurrentApp().Driver().CanvasForObject(s); c != nil {
		c.Focus(s) // clicking the screen sends the keyboard to the VM
	}
	if v := s.conn(); v != nil {
		fw, fh := v.Size()
		if _, _, ok := core.ToFrame(fw, fh, s.Size().Width, s.Size().Height, e.Position.X, e.Position.Y); !ok {
			return // a press in the black bars is not a click in macOS
		}
	}
	s.mu.Lock()
	s.buttons |= vncButton(e.Button)
	s.mu.Unlock()
	s.pointer(e.Position)
}

func (s *screen) MouseUp(e *desktop.MouseEvent) {
	s.mu.Lock()
	s.buttons &^= vncButton(e.Button)
	s.mu.Unlock()
	s.pointer(e.Position)
}

func (s *screen) MouseIn(e *desktop.MouseEvent)    { s.pointer(e.Position) }
func (s *screen) MouseMoved(e *desktop.MouseEvent) { s.pointer(e.Position) }

// MouseOut releases held buttons, so a drag that leaves the screen does not stay stuck.
func (s *screen) MouseOut() {
	s.mu.Lock()
	held, v := s.buttons, s.vnc
	s.buttons = 0
	s.mu.Unlock()
	if held != 0 && v != nil {
		v.Pointer(0, s.lastX, s.lastY)
	}
}

func (s *screen) wheel(dy float32) {
	v := s.conn()
	if v == nil || dy == 0 {
		return
	}
	bit := uint8(core.ButtonWheelDown)
	if dy > 0 {
		bit = core.ButtonWheelUp
	}
	s.mu.Lock()
	b, x, y := s.buttons, s.lastX, s.lastY
	s.mu.Unlock()
	v.Pointer(b|bit, x, y)
	v.Pointer(b, x, y)
}

// Cursor hides the host pointer over the screen while a VM is shown: macOS draws its own
// pointer into the framebuffer, so showing both would display two pointers.
func (s *screen) Cursor() desktop.Cursor {
	if s.conn() != nil {
		return desktop.HiddenCursor
	}
	return desktop.DefaultCursor
}

var (
	_ desktop.Cursorable = (*screen)(nil)
	_ fyne.Focusable     = (*screen)(nil)
	_ fyne.Tabbable      = (*screen)(nil)
	_ desktop.Keyable    = (*screen)(nil)
	_ desktop.Mouseable  = (*screen)(nil)
	_ desktop.Hoverable  = (*screen)(nil)
	_ fyne.Scrollable    = (*screen)(nil)
)

// typeLine opens the recovery's Terminal and types line there, as key presses (see
// core.GuestKeys). It returns at once; the keys follow at a typing pace.
func (s *screen) typeLine(line string) error {
	if !core.GuestTypingOK() {
		return errors.New("this keyboard layout moves the letters of the line (Dvorak, Russian): open Utilities › Terminal and type it yourself")
	}
	chords, err := core.GuestKeys(line)
	if err != nil {
		return err
	}
	s.mu.Lock()
	v := s.vnc
	s.mu.Unlock()
	if v == nil {
		return errors.New("the VM screen is not connected")
	}
	go func() {
		for _, c := range core.RecoveryTerminal() {
			if v.Chord(c) != nil {
				return
			}
			time.Sleep(120 * time.Millisecond) // the menu opens
		}
		time.Sleep(2 * time.Second) // the Terminal window opens
		for _, c := range chords {
			if v.Chord(c) != nil {
				return
			}
		}
	}()
	return nil
}

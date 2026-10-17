//go:build reims_embed

package embed

/*
#cgo CFLAGS: -I${SRCDIR}/../../reims-gui/crates/rg-x11-ffi
#cgo LDFLAGS: ${SRCDIR}/../../reims-gui/target/rg-x11-ffi/release/librg_x11_ffi.a -ldl -lpthread -lm
#include <stdlib.h>
#include "rg_x11.h"
*/
import "C"

import (
	"errors"
	"time"
	"unsafe"
)

const errLen = 512

// call runs f with an error buffer; the message comes back from the same cgo call (D1).
func call(f func(err *C.char, n C.size_t) C.int) error {
	buf := (*C.char)(C.malloc(errLen))
	defer C.free(unsafe.Pointer(buf))
	*buf = 0
	if f(buf, errLen) == 0 {
		return nil
	}
	msg := C.GoString(buf)
	if msg == "" {
		msg = "rg-x11: invalid argument"
	}
	return errors.New(msg)
}

func cstr(s string) (*C.char, func()) {
	if s == "" {
		return nil, func() {}
	}
	p := C.CString(s)
	return p, func() { C.free(unsafe.Pointer(p)) }
}

// Available reports whether embedding is built in.
func Available() bool { return true }

// Probe reports whether an X server answers on display ("" = $DISPLAY).
func Probe(display string) bool {
	d, free := cstr(display)
	defer free()
	return C.rgx_probe(d) == 1
}

// Embedder is a connection to the X server with the app's top-level window as the parent.
type Embedder struct{ p *C.RgxEmbedder }

// NewEmbedder connects to display ("" = $DISPLAY); parent is the app's X11 window.
func NewEmbedder(display string, parent uint32) (*Embedder, error) {
	d, free := cstr(display)
	defer free()
	var p *C.RgxEmbedder
	if err := call(func(e *C.char, n C.size_t) C.int {
		return C.rgx_embedder_new(d, C.uint32_t(parent), &p, e, n)
	}); err != nil {
		return nil, err
	}
	return &Embedder{p}, nil
}

// FindWindow blocks (polling) up to timeout for the window of QEMU process pid.
func (em *Embedder) FindWindow(pid uint32, k Kind, timeout time.Duration) (uint32, error) {
	var w C.uint32_t
	err := call(func(e *C.char, n C.size_t) C.int {
		return C.rgx_find_window(em.p, C.uint32_t(pid), C.int(k), C.uint32_t(timeout.Milliseconds()), &w, e, n)
	})
	return uint32(w), err
}

// Embed reparents child into a container over the parent window.
func (em *Embedder) Embed(child uint32) (*Embedded, error) {
	var p *C.RgxEmbedded
	if err := call(func(e *C.char, n C.size_t) C.int {
		return C.rgx_embed(em.p, C.uint32_t(child), &p, e, n)
	}); err != nil {
		return nil, err
	}
	return &Embedded{p}, nil
}

// Close frees the embedder; windows it embedded stay embedded.
func (em *Embedder) Close() {
	C.rgx_embedder_free(em.p)
	em.p = nil
}

// Embedded is QEMU's window inside the app's window.
type Embedded struct{ p *C.RgxEmbedded }

// SetRect moves and resizes it (physical pixels, relative to the app's window). Call it every
// frame: nothing is sent when the rect is unchanged.
func (h *Embedded) SetRect(x, y int32, w, ht uint32) error {
	return call(func(e *C.char, n C.size_t) C.int {
		return C.rgx_set_rect(h.p, C.int32_t(x), C.int32_t(y), C.uint32_t(w), C.uint32_t(ht), e, n)
	})
}

// Pump handles pending X events; never blocks.
func (h *Embedded) Pump() error {
	return call(func(e *C.char, n C.size_t) C.int { return C.rgx_pump(h.p, e, n) })
}

// Hide hides it (a dialog or menu over the tab, another tab).
func (h *Embedded) Hide() error {
	return call(func(e *C.char, n C.size_t) C.int { return C.rgx_hide(h.p, e, n) })
}

// Show shows it again.
func (h *Embedded) Show() error {
	return call(func(e *C.char, n C.size_t) C.int { return C.rgx_show(h.p, e, n) })
}

// Focus gives the guest the keyboard.
func (h *Embedded) Focus() error {
	return call(func(e *C.char, n C.size_t) C.int { return C.rgx_focus(h.p, e, n) })
}

// Unfocus gives the keyboard back to the app's window.
func (h *Embedded) Unfocus() error {
	return call(func(e *C.char, n C.size_t) C.int { return C.rgx_unfocus(h.p, e, n) })
}

// ReturnFocusOnAppClick gives the keyboard back to the app when a pointer button is held over
// the app's own UI while the guest has it (call it every tick). Whether it moved the keyboard.
func (h *Embedded) ReturnFocusOnAppClick() (bool, error) {
	var moved C.int
	err := call(func(e *C.char, n C.size_t) C.int { return C.rgx_return_focus_on_app_click(h.p, &moved, e, n) })
	return moved == 1, err
}

// Alive is false once QEMU's window is gone or another client took it.
func (h *Embedded) Alive() bool { return C.rgx_is_alive(h.p) == 1 }

// Release puts a live window back on the desktop and frees the handle.
func (h *Embedded) Release() error {
	p := h.p
	h.p = nil
	return call(func(e *C.char, n C.size_t) C.int { return C.rgx_release(p, e, n) })
}

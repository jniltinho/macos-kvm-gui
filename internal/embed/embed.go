// Package embed hosts QEMU's display window (Reims vGPU's SDL3 window) inside the app's window
// by X11 reparenting, through reims-gui's rg-x11 (crates/rg-x11-ffi, a C ABI). Built with
// `-tags reims_embed` (make build-go); without the tag a stub reports ErrUnavailable and the
// window stays standalone.
//
// An Embedded is not safe for concurrent use: one goroutine (the UI's) owns it. FindWindow
// blocks and should run on its own goroutine with its own Embedder.
package embed

import "errors"

// ErrUnavailable: the binary was built without the reims_embed tag.
var ErrUnavailable = errors.New("window embedding is not built in (build with -tags reims_embed)")

// Kind is the QEMU display window to look for.
type Kind int

const (
	// Reims is Reims vGPU's SDL3 window (title "Reims vGPU...").
	Reims Kind = 0
	// Qemu is QEMU's own window (title "QEMU...").
	Qemu Kind = 1
)

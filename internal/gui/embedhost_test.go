package gui

import (
	"testing"

	"fyne.io/fyne/v2"
)

func TestForceX11(t *testing.T) {
	for _, c := range []struct {
		avail         bool
		display, plat string
		want          bool
	}{
		{true, ":0", "", true},
		{true, ":0", "wayland", false}, // the user's choice wins
		{true, ":0", "x11", false},     // already set
		{true, "", "", false},          // no X server: stay native
		{false, ":0", "", false},       // built without embedding
	} {
		if got := forceX11(c.avail, c.display, c.plat); got != c.want {
			t.Errorf("forceX11(%v, %q, %q) = %v, want %v", c.avail, c.display, c.plat, got, c.want)
		}
	}
}

func TestPhysRect(t *testing.T) {
	for _, c := range []struct {
		scale float32
		x, y  int32
		w, h  uint32
	}{
		{1, 300, 120, 900, 600},
		{1.25, 375, 150, 1125, 750},
		{2, 600, 240, 1800, 1200},
	} {
		x, y, w, h := physRect(fyne.NewPos(300, 120), fyne.NewSize(900, 600), c.scale)
		if x != c.x || y != c.y || w != c.w || h != c.h {
			t.Errorf("scale %v: %d,%d %dx%d, want %d,%d %dx%d", c.scale, x, y, w, h, c.x, c.y, c.w, c.h)
		}
	}
	// Edges are rounded, not the size: two rects that touch in units touch in pixels.
	_, _, w1, _ := physRect(fyne.NewPos(0.3, 0), fyne.NewSize(10.4, 1), 1.5)
	x2, _, _, _ := physRect(fyne.NewPos(10.7, 0), fyne.NewSize(5, 1), 1.5)
	if int32(w1) != x2 {
		t.Errorf("gap or overlap: first ends at %d, second starts at %d", w1, x2)
	}
	if _, _, w, h := physRect(fyne.NewPos(0, 0), fyne.NewSize(0, 0), 1); w != 1 || h != 1 {
		t.Errorf("zero size must become 1x1 for X: %dx%d", w, h)
	}
}

func TestStaleLookup(t *testing.T) {
	if staleLookup(3, 3, 100, 100) {
		t.Error("the current run's lookup is not stale")
	}
	if !staleLookup(2, 3, 100, 100) {
		t.Error("a lookup from an earlier start is stale")
	}
	if !staleLookup(3, 3, 100, 0) {
		t.Error("a lookup whose QEMU exited is stale")
	}
}

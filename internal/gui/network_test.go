package gui

import (
	"image/png"
	"os"
	"path/filepath"
	"testing"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/test"
	"fyne.io/fyne/v2/widget"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

// Settings › Network with two VMs on 2222: the warning names the other VM, and "Use a free port"
// fills in the next free port and clears it. MKG_SCREENSHOT_DIR saves the page before and after.
func TestUseAFreePort(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	test.NewTempApp(t)
	mk := func(name string) core.VMEntry {
		c := core.Defaults()
		c.Name, c.SSHPort, c.Workspace = name, 2222, filepath.Join(t.TempDir(), name)
		c.Networks = core.NewNetworks(c.Version)
		e, err := core.CreateVM(c)
		if err != nil {
			t.Fatal(err)
		}
		return e
	}
	mk("Nilton")
	s := mk("sonoma-test")
	c := s.Config
	c.SSHPort = 2222 // shared, as Settings allows
	if err := core.SaveVM(s, c); err != nil {
		t.Fatal(err)
	}

	page, read := networkPage(s.ID, c, func() string { return c.Version })
	w := test.NewWindow(page)
	defer w.Close()
	w.Resize(fyne.NewSize(700, 520))
	var warn *widget.Label
	var free *widget.Button
	var walk func(o fyne.CanvasObject)
	walk = func(o fyne.CanvasObject) {
		switch v := o.(type) {
		case *widget.Label:
			if v.Text == "Also used by Nilton: they cannot run at the same time" {
				warn = v
			}
		case *widget.Button:
			if v.Text == "Use a free port" {
				free = v
			}
		case *fyne.Container:
			for _, c := range v.Objects {
				walk(c)
			}
		case *widget.Form:
			for _, it := range v.Items {
				walk(it.Widget)
			}
		}
	}
	walk(page)
	if warn == nil || warn.Hidden || free == nil {
		t.Fatalf("the shared-port warning and the button must show: %v %v", warn, free)
	}
	shot(t, w, "settings-network-shared.png")

	want, err := core.NextSSHPort(s.ID, nil)
	if err != nil {
		t.Fatal(err)
	}
	test.Tap(free)
	if _, port, err := read(); err != nil || port != want || port == 2222 {
		t.Fatalf("Use a free port: %d %v, want %d", port, err, want)
	}
	if !warn.Hidden {
		t.Fatal("the warning must go away")
	}
	shot(t, w, "settings-network-free.png")
}

func shot(t *testing.T, w fyne.Window, name string) {
	dir := os.Getenv("MKG_SCREENSHOT_DIR")
	if dir == "" {
		return
	}
	f, err := os.Create(filepath.Join(dir, name))
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	png.Encode(f, w.Canvas().Capture())
}

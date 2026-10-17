package gui

import (
	"fmt"
	"math"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/driver"
	"fyne.io/fyne/v2/driver/desktop"
	"fyne.io/fyne/v2/widget"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
	"github.com/jniltinho/macos-kvm-gui/internal/embed"
)

// reimsEmbed hosts Reims vGPU's window in the Screen tab (openspec embed-reims-window-go, design
// D3): found by QEMU's PID off the UI goroutine, then owned by the UI goroutine, which moves it
// over the tab, hides it under overlays and other tabs, and releases it when QEMU stops.
type reimsEmbed struct {
	u       *ui
	parent  uint32 // the app window's X11 id; 0 when it is not on X11 or embedding is not built in
	gen     int    // run generation: a lookup for an older run never embeds into a newer one
	pid     int
	h       *embed.Embedded
	shown   bool
	looking bool // a FindWindow for the current run is in flight
	stop    chan struct{}
}

const embedTimeout = 30 * time.Second

// appClass is the app window's X11 WM_CLASS (its Fyne app id), also the desktop entry's
// StartupWMClass.
const appClass = "io.github.jniltinho.macos-kvm-gui"

// forceX11 reports whether the GUI should set FYNE_PLATFORM=x11 before starting (design D4):
// with embedding built in and an X server to reach, unless the user chose a platform.
func forceX11(available bool, display, platform string) bool {
	return available && display != "" && platform == ""
}

// physRect converts a rect in Fyne units to X physical pixels at scale, rounding the edges (not
// the size) so adjacent rects never leave a gap or overlap (as rg_x11::to_physical).
func physRect(pos fyne.Position, size fyne.Size, scale float32) (x, y int32, w, h uint32) {
	r := func(v float32) float64 { return math.Round(float64(v * scale)) }
	x0, y0 := r(pos.X), r(pos.Y)
	x1, y1 := r(pos.X+size.Width), r(pos.Y+size.Height)
	return int32(x0), int32(y0), uint32(max(x1-x0, 1)), uint32(max(y1-y0, 1))
}

// staleLookup: a lookup result belongs to an older run (another start since) or a QEMU that
// is gone.
func staleLookup(gen, curGen, pid, curPid int) bool {
	return gen != curGen || pid != curPid
}

// x11Parent is the app window's X11 id (call on the UI goroutine), or 0.
func x11Parent(w fyne.Window) uint32 {
	var id uint32
	if nw, ok := w.(driver.NativeWindow); ok {
		nw.RunNative(func(ctx any) {
			if x, ok := ctx.(driver.X11WindowContext); ok {
				id = uint32(x.WindowHandle)
			}
		})
	}
	return id
}

// pending reports whether a window lookup is running for the current start.
func (e *reimsEmbed) pending() bool { return e.looking }

// enabled reports whether a Reims VM started now would be embedded.
func (e *reimsEmbed) enabled() bool {
	if !embed.Available() {
		return false
	}
	if e.parent == 0 {
		e.parent = x11Parent(e.u.win)
		if e.parent != 0 {
			// GLFW names the class after the title ("macOS KVM v0.11.0"): give window managers,
			// the desktop entry and GNOME's grab rules a stable one.
			if err := embed.SetWMClass("", e.parent, appClass); err != nil {
				e.u.appendLog("[VM] WM_CLASS: " + err.Error())
			}
		}
	}
	return e.parent != 0
}

// started runs after the app started a Reims VM with EmbedX11: it finds the window off the UI
// goroutine and embeds it on the UI goroutine.
func (e *reimsEmbed) started(pid int) {
	e.release()
	e.gen++
	e.pid = pid
	gen, parent, u := e.gen, e.parent, e.u
	u.screen.hint.SetText(screenReimsEmbedding)
	e.looking = true
	go func() {
		em, err := embed.NewEmbedder("", parent)
		var child uint32
		if err == nil {
			child, err = em.FindWindow(uint32(pid), embed.Reims, embedTimeout)
		}
		fyne.Do(func() {
			if em != nil {
				defer em.Close() // the Embedded keeps its own X connection
			}
			if staleLookup(gen, e.gen, pid, u.vm.Pid()) {
				return
			}
			e.looking = false
			if err == nil {
				e.h, err = em.Embed(child)
			}
			if err != nil {
				e.h = nil
				u.appendLog("[VM] embedding skipped: " + err.Error() + " (the guest stays in Reims vGPU's own window)")
				u.screen.hint.SetText(screenReims)
				u.refresh()
				return
			}
			u.appendLog(fmt.Sprintf("[VM] Reims vGPU's window (0x%x) is in the Screen tab", child))
			u.screen.hint.SetText("")
			u.refresh()
			e.shown = true
			e.tick()
			e.stop = make(chan struct{})
			go e.ticker(e.stop)
		})
	}()
}

func (e *reimsEmbed) ticker(stop chan struct{}) {
	t := time.NewTicker(time.Second / 60)
	defer t.Stop()
	for {
		select {
		case <-stop:
			return
		case <-t.C:
			fyne.Do(func() {
				select {
				case <-stop: // released while this tick was queued
				default:
					e.tick()
				}
			})
		}
	}
}

// tick (UI goroutine): follow the tab, hide under overlays and other tabs, pump X events and
// hand the keyboard back on clicks on the app's own UI.
func (e *reimsEmbed) tick() {
	h, u := e.h, e.u
	if h == nil {
		return
	}
	if !h.Alive() {
		e.release()
		return
	}
	c := u.win.Canvas()
	visible := u.tabs.SelectedIndex() == 1 && c.Overlays().Top() == nil
	if visible != e.shown {
		e.shown = visible
		var err error
		if visible {
			err = h.Show()
		} else {
			err = h.Hide()
		}
		if err != nil {
			u.appendLog("[VM] embedding: " + err.Error())
		}
	}
	pos := fyne.CurrentApp().Driver().AbsolutePositionForObject(u.screenHolder)
	x, y, w, ht := physRect(pos, u.screenHolder.Size(), c.Scale())
	err := h.SetRect(x, y, w, ht)
	if err == nil {
		err = h.Pump()
	}
	if err == nil && visible {
		_, err = h.ReturnFocusOnAppClick()
	}
	if err != nil {
		u.appendLog("[VM] embedding: " + err.Error())
		e.release()
	}
}

// release (UI goroutine) stops the ticker and puts a live window back on the desktop.
func (e *reimsEmbed) release() {
	if e.stop != nil {
		close(e.stop)
		e.stop = nil
	}
	if e.h != nil {
		if err := e.h.Release(); err != nil {
			e.u.appendLog("[VM] embedding release: " + err.Error())
		}
		e.h = nil
	}
	e.looking = false
	e.u.refresh()
}

// prepare sets up v for a Reims start from the GUI: the window on X11 when it will be
// embedded. CLI and MCP starts never come through here.
func (e *reimsEmbed) prepare(v *core.VM) {
	v.EmbedX11 = e.enabled()
}

// preferX11 sets FYNE_PLATFORM=x11 before the app starts when embedding needs it (design D4).
func preferX11() {
	if forceX11(embed.Available(), os.Getenv("DISPLAY"), os.Getenv("FYNE_PLATFORM")) {
		os.Setenv("FYNE_PLATFORM", "x11")
	}
}

// menuBar is the app-owned menu bar used while embedding is built in (design D6): Fyne's GLFW
// main menu draws its dropdowns outside the overlay stack, under the native guest window,
// while these buttons open PopUpMenus, which are overlays the embedding hides itself under.
// Alt+<first letter> opens a menu, as with the native bar.
func menuBar(w fyne.Window, mm *fyne.MainMenu) fyne.CanvasObject {
	row := container.NewHBox()
	for _, m := range mm.Items {
		m := m
		var b *widget.Button
		open := func() {
			c := w.Canvas()
			pos := fyne.CurrentApp().Driver().AbsolutePositionForObject(b).AddXY(0, b.Size().Height)
			widget.ShowPopUpMenuAtPosition(m, c, pos)
		}
		b = widget.NewButton(m.Label, open)
		b.Importance = widget.LowImportance
		row.Add(b)
		if r := []rune(m.Label); len(r) > 0 {
			key := fyne.KeyName(strings.ToUpper(string(r[0])))
			w.Canvas().AddShortcut(&desktop.CustomShortcut{KeyName: key, Modifier: fyne.KeyModifierAlt}, func(fyne.Shortcut) { open() })
		}
	}
	return row
}

// releaseOnSignals (design D5): on SIGTERM, SIGINT or SIGHUP the embedded window goes back to
// the desktop before the app exits, so the X server never sees a client die with a foreign
// window in its save-set (Xwayland 24.1 crashes then). QEMU still stops with the app
// (Pdeathsig), as before. If the UI goroutine is stuck, the process exits after 2 s anyway.
func releaseOnSignals(a fyne.App, u *ui) {
	c := make(chan os.Signal, 1)
	signal.Notify(c, syscall.SIGTERM, syscall.SIGINT, syscall.SIGHUP)
	go func() {
		<-c
		go func() {
			time.Sleep(2 * time.Second)
			os.Exit(143)
		}()
		fyne.DoAndWait(func() { u.emb.release() })
		a.Quit()
	}()
}

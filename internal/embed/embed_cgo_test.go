//go:build reims_embed

package embed

import (
	"bufio"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// sdlWindow is rg-x11's test helper (a real SDL3 window titled by its argument), built by
// `make test-embed`; RG_SDL_WINDOW overrides the path.
func sdlWindow(t *testing.T) string {
	p := os.Getenv("RG_SDL_WINDOW")
	if p == "" {
		p, _ = filepath.Abs("../../reims-gui/target/ubuntu24.04/debug/examples/sdl_window")
	}
	if _, err := os.Stat(p); err != nil {
		t.Skipf("no sdl_window helper (%v): run make test-embed", err)
	}
	return p
}

func xvfb(t *testing.T) string {
	if _, err := exec.LookPath("Xvfb"); err != nil {
		t.Skip("no Xvfb")
	}
	c := exec.Command("Xvfb", "-noreset", "-displayfd", "1", "-screen", "0", "1280x800x24", "-nolisten", "tcp")
	out, err := c.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := c.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = c.Process.Kill(); _ = c.Wait() })
	line, err := bufio.NewReader(out).ReadString('\n')
	if err != nil {
		t.Fatal(err)
	}
	d := ":" + strings.TrimSpace(line)
	for i := 0; i < 50 && !Probe(d); i++ {
		time.Sleep(100 * time.Millisecond)
	}
	return d
}

func window(t *testing.T, helper, display, title string) *exec.Cmd {
	c := exec.Command(helper, title)
	c.Env = append(os.Environ(), "DISPLAY="+display, "SDL_VIDEO_DRIVER=x11")
	if err := c.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = c.Process.Kill(); _ = c.Wait() })
	return c
}

// windowID is the id of the first window titled title (xdotool).
func windowID(t *testing.T, display, title string) uint32 {
	for i := 0; i < 100; i++ {
		c := exec.Command("xdotool", "search", "--name", "^"+title+"$")
		c.Env = append(os.Environ(), "DISPLAY="+display)
		if out, err := c.Output(); err == nil {
			var id uint32
			if _, err := fmt.Sscan(strings.Fields(string(out))[0], &id); err == nil {
				return id
			}
		}
		time.Sleep(100 * time.Millisecond)
	}
	t.Fatalf("no window %q", title)
	return 0
}

func parentOf(t *testing.T, display string, w uint32) string {
	c := exec.Command("xwininfo", "-children", "-id", fmt.Sprint(w))
	c.Env = append(os.Environ(), "DISPLAY="+display)
	out, err := c.Output()
	if err != nil {
		t.Fatalf("xwininfo: %v", err)
	}
	for _, l := range strings.Split(string(out), "\n") {
		if strings.Contains(l, "Parent window id:") {
			return strings.Fields(strings.SplitN(l, ":", 2)[1])[0]
		}
	}
	return ""
}

// The whole wrapper against a real X server: an "app" window and a Reims-like guest window,
// found by PID, embedded, moved, hidden, shown and released back to the root.
func TestEmbedReimsWindow(t *testing.T) {
	helper := sdlWindow(t)
	for _, tool := range []string{"xdotool", "xwininfo", "xprop"} {
		if _, err := exec.LookPath(tool); err != nil {
			t.Skipf("no %s", tool)
		}
	}
	d := xvfb(t)
	window(t, helper, d, "Fake app")
	guest := window(t, helper, d, "Reims vGPU (port 2)")
	app := windowID(t, d, "Fake app")
	rootParent := parentOf(t, d, app)

	if err := SetWMClass(d, app, "io.test.embed"); err != nil {
		t.Fatal(err)
	}
	c := exec.Command("xprop", "-id", fmt.Sprint(app), "WM_CLASS")
	c.Env = append(os.Environ(), "DISPLAY="+d)
	if out, err := c.Output(); err != nil || !strings.Contains(string(out), `"io.test.embed", "io.test.embed"`) {
		t.Fatalf("WM_CLASS: %s %v", out, err)
	}

	em, err := NewEmbedder(d, app)
	if err != nil {
		t.Fatal(err)
	}
	defer em.Close()
	child, err := em.FindWindow(uint32(guest.Process.Pid), Reims, 10*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := em.FindWindow(uint32(guest.Process.Pid), Kind(7), 10*time.Millisecond); err == nil ||
		!strings.Contains(err.Error(), "unknown window kind") {
		t.Fatalf("an invalid kind must fail with the library's message: %v", err)
	}
	h, err := em.Embed(child)
	if err != nil {
		t.Fatal(err)
	}
	for _, step := range []func() error{
		func() error { return h.SetRect(10, 20, 300, 200) },
		h.Pump, h.Hide, h.Show, h.Pump,
	} {
		if err := step(); err != nil {
			t.Fatal(err)
		}
	}
	if !h.Alive() {
		t.Fatal("embedded window not alive")
	}
	container := parentOf(t, d, child)
	if container == rootParent || parentOf(t, d, mustHex(t, container)) != fmt.Sprintf("0x%x", app) {
		t.Fatalf("not embedded under the app: child's parent %s", container)
	}
	if err := h.Release(); err != nil {
		t.Fatal(err)
	}
	if got := parentOf(t, d, child); got != rootParent {
		t.Fatalf("not released to the root: parent %s, want %s", got, rootParent)
	}
}

func mustHex(t *testing.T, s string) uint32 {
	var v uint32
	if _, err := fmt.Sscanf(s, "0x%x", &v); err != nil {
		t.Fatalf("window id %q: %v", s, err)
	}
	return v
}

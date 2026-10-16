//go:build integration

// Test with real QEMU and KVM: go test -tags integration -run Real ./internal/core
// Boots the embedded OpenCore through vvfat in a temporary workspace.
package core

import (
	"context"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"
)

func waitState(t *testing.T, v *VM, want VMState) {
	t.Helper()
	for range 100 {
		if v.State() == want {
			return
		}
		time.Sleep(100 * time.Millisecond)
	}
	t.Fatalf("state %v, expected %v", v.State(), want)
}

func TestRealQEMULifecycle(t *testing.T) {
	w := NewWorkspace(filepath.Join(t.TempDir(), "ws,with,comma"))
	os.Mkdir(w.Dir, 0o755)
	if _, err := w.CreateDisk(context.Background(), 40); err != nil {
		t.Fatal(err)
	}
	c := Defaults()
	c.Workspace = w.Dir
	c.SSHPort = 22299

	var mu sync.Mutex
	var msgs []string
	v := &VM{OnState: func(_ VMState, m string) { mu.Lock(); msgs = append(msgs, m); mu.Unlock() }, OnLog: func(s string) { t.Log(s) }}
	if err := v.Start(c); err != nil {
		t.Fatal(err)
	}
	waitState(t, v, Running)
	if !exists(w.Path(VarsFile)) {
		t.Fatal("the NVRAM was not created")
	}
	if _, err := w.Lock(); err != ErrBusy {
		t.Fatalf("workspace must stay locked while the VM runs: %v", err)
	}
	if err := v.Start(c); err == nil {
		t.Fatal("second start must be refused")
	}
	// The in-app display: QEMU must serve the boot screen over the private VNC socket.
	if v.VNCSocket() == "" {
		t.Fatal("the default display must export VNC")
	}
	vnc, verr := DialVNC(v.VNCSocket())
	if verr != nil {
		t.Fatalf("VNC: %v", verr)
	}
	frames := make(chan struct{}, 64)
	vnc.OnUpdate = func() {
		select {
		case frames <- struct{}{}:
		default:
		}
	}
	go vnc.Run()
	lit := false
	for deadline := time.After(20 * time.Second); !lit; {
		select {
		case <-frames:
			f := vnc.Frame(nil)
			for i := 0; i < len(f.Pix); i += 4 {
				if f.Pix[i]|f.Pix[i+1]|f.Pix[i+2] != 0 {
					lit = true // OVMF or OpenCore drew something
					break
				}
			}
		case <-deadline:
			t.Fatal("no non-black frame over VNC within 20 s")
		}
	}
	fw, fh := vnc.Size()
	t.Logf("VNC frame %dx%d", fw, fh)
	vnc.Close()

	// QMP connects in the background; QEMU must accept the powerdown
	var err error
	for range 50 {
		if err = v.Powerdown(); err == nil {
			break
		}
		time.Sleep(100 * time.Millisecond)
	}
	if err != nil {
		t.Fatalf("powerdown via QMP: %v", err)
	}
	if v.State() != Running {
		t.Fatal("powerdown must not change the state by itself")
	}
	if err := v.Kill(); err != nil {
		t.Fatal(err)
	}
	waitState(t, v, Stopped)
	unlock, err := w.Lock()
	if err != nil {
		t.Fatalf("lock must be released when QEMU exits: %v", err)
	}
	unlock()
	time.Sleep(100 * time.Millisecond) // OnState(Stopped) runs right after the state changes
	mu.Lock()
	defer mu.Unlock()
	if last := msgs[len(msgs)-1]; last != "Stopped by the user (forced)." {
		t.Fatalf("a requested forced stop is not an error: %q", last)
	}
}

//go:build integration

// Test with real QEMU and KVM: go test -tags integration -run Real ./internal/core
// Requires an OSX-KVM clone at $OSX_KVM (default ~/OSX-KVM), used read-only.
package core

import (
	"context"
	"io"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"
)

func copyFile(t *testing.T, src, dst string) {
	t.Helper()
	os.MkdirAll(filepath.Dir(dst), 0o755)
	in, err := os.Open(src)
	if err != nil {
		t.Skip("OSX-KVM not found: ", err)
	}
	defer in.Close()
	out, err := os.Create(dst)
	if err != nil {
		t.Fatal(err)
	}
	defer out.Close()
	io.Copy(out, in)
}

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
	src := os.Getenv("OSX_KVM")
	if src == "" {
		home, _ := os.UserHomeDir()
		src = filepath.Join(home, "OSX-KVM")
	}
	w := NewWorkspace(filepath.Join(t.TempDir(), "ws,with,comma"))
	for _, f := range RequiredFiles {
		copyFile(t, filepath.Join(src, f), w.Path(f))
	}
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
		t.Fatal("the app's own NVRAM was not created")
	}
	if _, err := w.Lock(); err != ErrBusy {
		t.Fatalf("workspace must stay locked while the VM runs: %v", err)
	}
	if err := v.Start(c); err == nil {
		t.Fatal("second start must be refused")
	}
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

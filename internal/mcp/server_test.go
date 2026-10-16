package mcp

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

func session(t *testing.T) *mcp.ClientSession {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	if err := core.SetWorkspaceRoot(t.TempDir()); err != nil {
		t.Fatal(err)
	}
	st, ct := mcp.NewInMemoryTransports()
	ctx := context.Background()
	if _, err := newServer("test").Connect(ctx, st, nil); err != nil {
		t.Fatal(err)
	}
	cs, err := mcp.NewClient(&mcp.Implementation{Name: "test"}, nil).Connect(ctx, ct, nil)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { cs.Close() })
	return cs
}

func call(t *testing.T, cs *mcp.ClientSession, name string, args map[string]any) (string, bool) {
	r, err := cs.CallTool(context.Background(), &mcp.CallToolParams{Name: name, Arguments: args})
	if err != nil {
		t.Fatalf("%s: %v", name, err)
	}
	var b strings.Builder
	for _, c := range r.Content {
		if tc, ok := c.(*mcp.TextContent); ok {
			b.WriteString(tc.Text)
		}
	}
	return b.String(), r.IsError
}

func TestTools(t *testing.T) {
	cs := session(t)
	tools, err := cs.ListTools(context.Background(), nil)
	if err != nil {
		t.Fatal(err)
	}
	got := map[string]*mcp.Tool{}
	for _, tl := range tools.Tools {
		got[tl.Name] = tl
	}
	for _, n := range []string{"vm_list", "vm_status", "vm_start", "vm_shutdown", "vm_force_stop", "screen_capture", "send_keys",
		"type_text", "pointer", "installer_list", "installer_download", "installer_remove", "install_offline", "host_check"} {
		if got[n] == nil {
			t.Errorf("missing tool %s", n)
		}
	}
	for _, n := range []string{"vm_force_stop", "installer_remove", "install_offline"} {
		if a := got[n].Annotations; a == nil || a.DestructiveHint == nil || !*a.DestructiveHint {
			t.Errorf("%s must be marked destructive", n)
		}
	}
}

// Consent and prerequisites are checked before anything happens: no VM even exists here.
func TestConsentFirst(t *testing.T) {
	cs := session(t)
	for _, c := range []struct {
		tool string
		args map[string]any
		want string
	}{
		{"vm_force_stop", map[string]any{}, "confirm: true"},
		{"installer_remove", map[string]any{"build": "24H32"}, "confirm: true"},
		{"install_offline", map[string]any{"unattended": true, "erase_disk": true}, "license"},
		{"install_offline", map[string]any{"unattended": true, "accept_license": true}, "erase"},
		{"type_text", map[string]any{"text": "a\nb"}, "line break"},
		{"send_keys", map[string]any{"keys": []string{"shift+nope"}}, "unknown key"},
		{"installer_download", map[string]any{"version": "catalina"}, "Big Sur"},
		{"vm_status", map[string]any{"vm": "nope"}, "no VM"},
	} {
		msg, isErr := call(t, cs, c.tool, c.args)
		if !isErr || !strings.Contains(msg, c.want) {
			t.Errorf("%s %v: %q (error %v), want %q", c.tool, c.args, msg, isErr, c.want)
		}
	}
	if msg, isErr := call(t, cs, "installer_list", nil); isErr {
		t.Errorf("installer_list: %s", msg)
	}
}

func TestOverallProgressOnlyGrows(t *testing.T) {
	last := -1.0
	for _, p := range []core.Progress{{Stage: "Reading Apple's software catalog", Pct: -1},
		{Stage: "Downloading the macOS 15 installer", Pct: 0}, {Stage: "Downloading the macOS 15 installer", Pct: 100},
		{Stage: "Checking the package", Pct: 0}, {Stage: "Checking the package", Pct: 100}, {Stage: "Writing the installer disc", Pct: -1}} {
		v := overall(p)
		if v < last || v > 100 {
			t.Fatalf("%v gives %v after %v", p, v, last)
		}
		last = v
	}
}

// vm_start downloads the extra kexts itself, before spawning the VM and its 30 s wait.
func TestVMStartDownloadsKextsFirst(t *testing.T) {
	cs := session(t)
	c := core.Defaults()
	c.Name, c.Version = "Lab", "sequoia"
	c.Workspace = filepath.Join(t.TempDir(), "lab")
	c.Networks = core.NewNetworks(c.Version)
	e, err := core.CreateVM(c)
	if err != nil {
		t.Fatal(err)
	}
	os.MkdirAll(filepath.Dir(core.SettingsFile()), 0o755)
	os.WriteFile(core.SettingsFile(), []byte("kexts:\n  - url: https://127.0.0.1:1/x.zip\n"), 0o644)
	msg, isErr := call(t, cs, "vm_start", map[string]any{"vm": e.ID})
	if !isErr || !strings.Contains(msg, "127.0.0.1:1") {
		t.Fatalf("vm_start: %q (error %v)", msg, isErr)
	}
	if _, err := os.Stat(filepath.Join(c.Workspace, "macos-kvm-gui", "mcp-start.log")); err == nil {
		t.Fatal("no VM process is spawned when a kext cannot be downloaded")
	}
}

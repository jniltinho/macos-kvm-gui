package core

import (
	"errors"
	"path/filepath"
	"strings"
	"testing"
)

func TestConfigRoundTripAndDefaults(t *testing.T) {
	path := filepath.Join(t.TempDir(), "sub", "config.json")
	c, err := LoadConfig(path)
	if err != nil || c != Defaults() {
		t.Fatalf("no file should yield defaults: %+v %v", c, err)
	}
	c.RAMMiB = 16384
	if err := SaveConfig(path, c); err != nil {
		t.Fatal(err)
	}
	got, err := LoadConfig(path)
	if err != nil || got.RAMMiB != 16384 {
		t.Fatalf("round trip failed: %+v %v", got, err)
	}
}

func TestConfigValidate(t *testing.T) {
	cases := map[string]func(*Config){
		"ram":       func(c *Config) { c.RAMMiB = 2048 },
		"cores":     func(c *Config) { c.Cores = 0 },
		"vcpus":     func(c *Config) { c.Cores, c.VCPUs = 4, 6 },
		"ssh_port":  func(c *Config) { c.SSHPort = 80 },
		"workspace": func(c *Config) { c.Workspace = "relative/dir" },
		"version":   func(c *Config) { c.Version = "lion" },
	}
	for field, mutate := range cases {
		c := Defaults()
		mutate(&c)
		var fe *FieldError
		if err := c.Validate(); !errors.As(err, &fe) || fe.Field != field {
			t.Errorf("%s: expected a FieldError for the field, got %v", field, err)
		}
	}
	if err := Defaults().Validate(); err != nil {
		t.Fatalf("defaults must be valid: %v", err)
	}
}

func TestRecommended(t *testing.T) {
	cases := []struct{ threads, memMiB, vcpus, cores, ram int }{
		{24, 186 * 1024, 12, 6, 16384}, // reference host
		{4, 8 * 1024, 2, 1, 4096},
		{64, 512 * 1024, 16, 8, 16384},
		{6, 24 * 1024, 2, 1, 6144}, // 6/2=3 rounds down to even: 2
	}
	for _, tc := range cases {
		c := Recommended(Defaults(), tc.threads, tc.memMiB)
		if c.VCPUs != tc.vcpus || c.Cores != tc.cores || c.RAMMiB != tc.ram {
			t.Errorf("%d threads/%d MiB: got %d vCPUs, %d cores, %d MiB", tc.threads, tc.memMiB, c.VCPUs, c.Cores, c.RAMMiB)
		}
		if err := c.Validate(); err != nil {
			t.Errorf("invalid recommendation: %v", err)
		}
	}
}

func TestWorkspaceTilde(t *testing.T) {
	t.Setenv("HOME", "/home/x")
	d, err := Defaults().WorkspaceDir()
	if err != nil || d != "/home/x/OSX-KVM" {
		t.Fatalf("got %q %v", d, err)
	}
}

func TestProfileFor(t *testing.T) {
	for _, v := range Versions {
		p := ProfileFor(v.Short)
		old := versionIndex(v.Short) <= idxMonterey
		if old != strings.HasPrefix(p.CPU, "Penryn") {
			t.Errorf("%s: CPU %s", v.Short, p.CPU)
		}
		if p.NeedsAVX2 == old {
			t.Errorf("%s: NeedsAVX2=%v", v.Short, p.NeedsAVX2)
		}
		if (v.Short == "high-sierra") != (p.NIC == "vmxnet3") {
			t.Errorf("%s: NIC %s", v.Short, p.NIC)
		}
		if p.Experimental != (v.Short != "sonoma") {
			t.Errorf("%s: experimental %v", v.Short, p.Experimental)
		}
	}
}

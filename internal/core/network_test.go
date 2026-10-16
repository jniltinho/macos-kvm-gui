package core

import (
	"database/sql"
	"encoding/json"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestLegacyVMKeepsItsNetwork(t *testing.T) {
	c := Defaults() // Sonoma, no adapter list
	first, rest := netArgs(c, "/usr/lib/qemu/qemu-bridge-helper")
	want := []string{"-netdev", "user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22",
		"-device", "virtio-net-pci,netdev=net0,id=net0,mac=52:54:00:c9:18:27"}
	if strings.Join(first, " ") != strings.Join(want, " ") || len(rest) != 0 {
		t.Fatalf("legacy arguments changed:\n%v\n%v", first, rest)
	}
}

func TestNetArgs(t *testing.T) {
	c := Defaults()
	c.Networks = []NIC{
		{Enabled: true, Mode: NetNAT, Model: "virtio-net-pci", MAC: "52:54:00:00:00:01"},
		{Enabled: true, Mode: NetBridged, Bridge: "br0", Model: "vmxnet3", MAC: "52:54:00:00:00:02"},
		{Enabled: false, Mode: NetNAT, Model: "vmxnet3", MAC: "52:54:00:00:00:03"},
		{Enabled: true, Mode: NetNAT, Model: "vmxnet3", MAC: "52:54:00:00:00:04"},
	}
	first, rest := netArgs(c, "/usr/lib/qemu/qemu-bridge-helper")
	all := strings.Join(append(first, rest...), " ")
	for _, s := range []string{
		"user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22",
		"bridge,id=net1,br=br0,helper=/usr/lib/qemu/qemu-bridge-helper",
		"vmxnet3,netdev=net1,id=net1,mac=52:54:00:00:00:02,bus=pcie.0,addr=0x16",
		"user,id=net3,net=10.0.5.0/24", // a second NAT adapter: own subnet, no second forward
		"vmxnet3,netdev=net3,id=net3,mac=52:54:00:00:00:04,bus=pcie.0,addr=0x18",
	} {
		if !strings.Contains(all, s) {
			t.Errorf("missing %q in %s", s, all)
		}
	}
	if strings.Contains(all, "net2") || strings.Count(all, "hostfwd") != 1 {
		t.Errorf("disabled adapters stay out, and only one forward: %s", all)
	}

	c.Networks = []NIC{{Enabled: false, Mode: NetNAT, Model: "vmxnet3", MAC: "52:54:00:00:00:01"}}
	if first, _ := netArgs(c, ""); strings.Join(first, " ") != "-nic none" || c.SSHForwarded() {
		t.Fatalf("no enabled adapter must give -nic none: %v", first)
	}
}

func TestValidateNICs(t *testing.T) {
	ok := NIC{Enabled: true, Mode: NetNAT, Model: "vmxnet3", MAC: "52:54:00:12:34:56"}
	cases := map[string][]NIC{
		"multicast":   {{Enabled: true, Mode: NetNAT, Model: "vmxnet3", MAC: "01:00:5e:00:00:01"}},
		"global":      {{Enabled: true, Mode: NetNAT, Model: "vmxnet3", MAC: "00:1c:42:00:00:01"}},
		"format":      {{Enabled: true, Mode: NetNAT, Model: "vmxnet3", MAC: "5254.0012.3456"}},
		"duplicate":   {ok, ok},
		"model":       {{Enabled: true, Mode: NetNAT, Model: "rtl8139", MAC: "52:54:00:12:34:56"}},
		"bridge name": {{Enabled: true, Mode: NetBridged, Bridge: "br0;rm", Model: "vmxnet3", MAC: "52:54:00:12:34:56"}},
		"too many":    {ok, ok, ok, ok, ok},
	}
	for name, nics := range cases {
		if err := validateNICs(nics); err == nil {
			t.Errorf("%s must be refused", name)
		}
	}
	if err := validateNICs([]NIC{ok}); err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 50; i++ {
		hw, _ := net.ParseMAC(RandomMAC())
		if hw[0]&3 != 2 {
			t.Fatalf("random MAC %s is not locally administered unicast", hw)
		}
	}
}

func TestNICSupported(t *testing.T) {
	for _, c := range []struct {
		version, model string
		ok             bool
	}{
		{"catalina", "virtio-net-pci", false}, {"big-sur", "virtio-net-pci", true},
		{"monterey", "e1000-82545em", true}, {"ventura", "e1000-82545em", false},
		{"high-sierra", "vmxnet3", true}, {"tahoe", "vmxnet3", true},
	} {
		if NICSupported(c.version, c.model) != c.ok {
			t.Errorf("%s on %s: want %v", c.model, c.version, c.ok)
		}
	}
	if defaultNICModel("mojave") != "vmxnet3" || defaultNICModel("sonoma") != "virtio-net-pci" {
		t.Error("defaults must follow the driver matrix")
	}
}

func TestBridgeACL(t *testing.T) {
	dir := t.TempDir()
	acl := filepath.Join(dir, "bridge.conf")
	extra := filepath.Join(dir, "extra.conf")
	write(t, extra, "allow br1\n")
	write(t, acl, "# comment\nallow br0\ninclude "+extra+"\ndeny br2\nallow all\n")
	for br, want := range map[string]bool{"br0": true, "br1": true, "br2": false, "virbr0": true} {
		if got, known := bridgeAllowed(acl, br); !known || got != want {
			t.Errorf("%s: got %v (known %v), want %v", br, got, known, want)
		}
	}
	if _, known := bridgeAllowed(filepath.Join(dir, "missing"), "br0"); known {
		t.Error("an unreadable ACL leaves the decision to the helper")
	}

	netRoot := t.TempDir()
	os.MkdirAll(filepath.Join(netRoot, "br0", "bridge"), 0o755)
	os.MkdirAll(filepath.Join(netRoot, "eth0"), 0o755)
	c := Defaults()
	c.Networks = []NIC{{Enabled: true, Mode: NetBridged, Bridge: "br2", Model: "vmxnet3", MAC: "52:54:00:00:00:01"}}
	write(t, acl, "allow br0\n")
	if err := checkBridges(c, netRoot, acl, "/helper"); err == nil || !strings.Contains(err.Error(), "no bridge named br2") {
		t.Errorf("a missing bridge must be refused: %v", err)
	}
	c.Networks[0].Bridge = "br0"
	if err := checkBridges(c, netRoot, acl, ""); err == nil || !strings.Contains(err.Error(), "qemu-bridge-helper") {
		t.Errorf("a missing helper must be refused: %v", err)
	}
	write(t, acl, "deny br0\nallow all\n")
	if err := checkBridges(c, netRoot, acl, "/helper"); err == nil || !strings.Contains(err.Error(), `allow br0`) {
		t.Errorf("a denied bridge must be refused with the fix: %v", err)
	}
	write(t, acl, "allow br0\n")
	if err := checkBridges(c, netRoot, acl, "/helper"); err != nil {
		t.Fatal(err)
	}
	if got := bridgesIn(netRoot); len(got) != 1 || got[0] != "br0" {
		t.Errorf("bridges: %v", got)
	}
}

func TestSSHPortInUse(t *testing.T) {
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Skip(err)
	}
	defer l.Close()
	c := Defaults()
	c.SSHPort = l.Addr().(*net.TCPAddr).Port
	if err := checkSSHPort(c); err == nil {
		t.Fatal("a taken port must be reported before QEMU starts")
	}
}

func TestMigrationToVersion2(t *testing.T) {
	cfg := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", cfg)
	path, _ := DBPath()
	os.MkdirAll(filepath.Dir(path), 0o755)
	// A version 1 database, as the previous release left it.
	old, err := sql.Open("sqlite", "file:"+path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := old.Exec(schemaV1 + `PRAGMA user_version = 1;
		INSERT INTO vms (id, name, workspace, version, ram_mib, cores, vcpus, ssh_port) VALUES ('old', 'Old', '/tmp/old', 'sonoma', 8192, 4, 8, 2222);`); err != nil {
		t.Fatal(err)
	}
	old.Close()

	vms, err := ListVMs()
	if err != nil || len(vms) != 1 || vms[0].Config.Networks != nil {
		t.Fatalf("the old VM must survive with no adapter list: %v %+v", err, vms)
	}
	if a := vms[0].Config.Adapters(); len(a) != 1 || a[0].MAC != legacyMAC {
		t.Fatalf("legacy adapter: %+v", a)
	}
	db, _ := openDB()
	var v int
	db.QueryRow(`PRAGMA user_version`).Scan(&v)
	db.Close()
	if v != 2 {
		t.Fatalf("user_version = %d", v)
	}

	c := vms[0].Config
	c.Networks = NewNetworks("sonoma")
	if err := SaveVM(vms[0], c); err != nil {
		t.Fatal(err)
	}
	if again, _ := ListVMs(); len(again[0].Config.Networks) != 1 || again[0].Config.Networks[0].MAC != c.Networks[0].MAC {
		t.Fatalf("adapters must persist: %+v", again[0].Config.Networks)
	}
}

func TestDiskVirtualSize(t *testing.T) {
	p := filepath.Join(t.TempDir(), "d.img")
	qcow2(t, p, "")
	if got := DiskVirtualSize(p); got != 64<<20 {
		t.Fatalf("virtual size %d, want %d", got, 64<<20)
	}
	if DiskVirtualSize(filepath.Join(t.TempDir(), "none")) != 0 {
		t.Fatal("a missing disk has no size")
	}
}

func TestNoAdaptersSurviveJSON(t *testing.T) {
	c := Defaults()
	c.Networks = []NIC{}
	b, err := json.Marshal(c)
	if err != nil {
		t.Fatal(err)
	}
	var back Config
	if err := json.Unmarshal(b, &back); err != nil || back.Networks == nil || len(back.Networks) != 0 {
		t.Fatalf("an empty adapter list must not turn back into the legacy adapter: %s", b)
	}
	var legacy Config
	json.Unmarshal([]byte(`{"version":"sonoma"}`), &legacy)
	if legacy.Networks != nil {
		t.Fatal("an older file without networks stays legacy")
	}
}

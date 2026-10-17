package core

import (
	"database/sql"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"testing"
	"time"
)

// TestBuildArgsSVGAGolden pins the vmware-svga command: Reims vGPU must not change it.
// MKG_UPDATE_GOLDEN=1 rewrites testdata/buildargs-svga.golden.
func TestBuildArgsSVGAGolden(t *testing.T) {
	w := NewWorkspace("/vm/a,b")
	sock := "/run/macos-kvm-gui-x/qmp.sock"
	c := Defaults()
	app := strings.Join(BuildArgs(c, w, sock, false, nil), "\n")
	c.Graphics, c.Display = GraphicsSVGA, DisplayWindow
	win := strings.Join(BuildArgs(c, w, sock, false, []string{"0000:03:00.0", "0000:03:00.1"}), "\n")
	got := app + "\n--\n" + win + "\n"
	golden := filepath.Join("testdata", "buildargs-svga.golden")
	if os.Getenv("MKG_UPDATE_GOLDEN") == "1" {
		if err := os.WriteFile(golden, []byte(got), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	want, err := os.ReadFile(golden)
	if err != nil {
		t.Fatal(err)
	}
	if got != string(want) {
		t.Fatalf("vmware-svga arguments changed:\n%s\nwant:\n%s", got, want)
	}
}

func withAudio(t *testing.T, backend string) {
	old := reimsAudio
	reimsAudio = func() string { return backend }
	t.Cleanup(func() { reimsAudio = old })
}

func TestBuildArgsReims(t *testing.T) {
	withAudio(t, "pipewire")
	c := Defaults()
	c.Graphics, c.Display, c.RAMMiB = GraphicsReims, DisplayWindow, 16384
	args := BuildArgs(c, NewWorkspace("/vm/a"), "/run/macos-kvm-gui-x/qmp.sock", false, nil)
	joined := strings.Join(args, " ")
	for _, want := range []string{
		"-m 16384 ", "-machine q35,memory-backend=ram -object memory-backend-memfd,id=ram,size=16384M,share=on",
		"-global ICH9-LPC.disable_s3=1", "-vga none",
		"-device apple-gfx-pci,id=gfx,displays=1,resolution=1920x1080 ",
		"-device qemu-xhci,id=xhci,conditional-intr-mapping=on ",
		"-device hda-duplex,audiodev=snd0 -audiodev pipewire,id=snd0 -device usb-audio,audiodev=snd0,bus=xhci.0",
		"-display none -vnc unix:/run/macos-kvm-gui-x/vnc.sock,lock-key-sync=off",
	} {
		if !strings.Contains(joined, want) {
			t.Errorf("missing %q in\n%s", want, joined)
		}
	}
	if strings.Contains(joined, "vmware-svga") || strings.Contains(joined, "addr=") || strings.Count(joined, "-audiodev") != 1 {
		t.Errorf("no vmware-svga, no fixed PCI address, one audiodev:\n%s", joined)
	}
	if r, ok := runningInArgs(args); !ok || r != "/run/macos-kvm-gui-x/vnc.sock" {
		t.Errorf("running-VM detection must still find the VNC socket: %q", r)
	}
}

// runningInArgs is the -vnc parsing of runningIn, over a fake /proc.
func runningInArgs(args []string) (string, bool) {
	proc, _ := os.MkdirTemp("", "proc-")
	defer os.RemoveAll(proc)
	c := Defaults()
	c.Workspace = proc
	os.WriteFile(filepath.Join(proc, DiskFile), nil, 0o644)
	os.MkdirAll(filepath.Join(proc, "42"), 0o755)
	// /proc/<pid>/comm of the Reims build, whose binary is also named qemu-system-x86_64.
	os.WriteFile(filepath.Join(proc, "42", "comm"), []byte("qemu-system-x86\n"), 0o644)
	args = append(slices.Clone(args), "-drive", "file="+filepath.Join(proc, DiskFile))
	os.WriteFile(filepath.Join(proc, "42", "cmdline"), []byte(strings.Join(args, "\x00")+"\x00"), 0o644)
	r, ok := runningIn(proc, c)
	return r.VNCSocket, ok
}

func TestGraphicsValidate(t *testing.T) {
	c := Defaults()
	var fe *FieldError
	for _, g := range []string{"", GraphicsSVGA, GraphicsReims} {
		c.Graphics = g
		if err := c.Validate(); err != nil {
			t.Errorf("%q: %v", g, err)
		}
	}
	c.Graphics = "virtio"
	if err := c.Validate(); !errors.As(err, &fe) || fe.Field != "graphics" || !strings.Contains(err.Error(), GraphicsReims) {
		t.Errorf("invalid value: %v", err)
	}
	c.Graphics, c.GPU = GraphicsReims, "0000:03:00.0"
	if err := c.Validate(); !errors.As(err, &fe) || fe.Field != "graphics" || !strings.Contains(err.Error(), "choose one") {
		t.Errorf("Reims vGPU with a GPU: %v", err)
	}
}

func TestGraphicsSchema4(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	path, _ := DBPath()
	os.MkdirAll(filepath.Dir(path), 0o755)
	// A version 3 database, as the previous release left it.
	old, err := sql.Open("sqlite", "file:"+path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := old.Exec(schemaV1 + `ALTER TABLE vms ADD COLUMN networks TEXT NOT NULL DEFAULT '';
		ALTER TABLE vms ADD COLUMN root_patch INTEGER NOT NULL DEFAULT 0;
		PRAGMA user_version = 3;
		INSERT INTO vms (id, name, workspace, version, ram_mib, cores, vcpus, ssh_port) VALUES ('old', 'Old', '/tmp/old', 'sequoia', 8192, 4, 8, 2222);`); err != nil {
		t.Fatal(err)
	}
	old.Close()

	vms, err := ListVMs()
	if err != nil || len(vms) != 1 || vms[0].Config.Graphics != "" || vms[0].Config.Reims() {
		t.Fatalf("an old VM stays on vmware-svga: %v %+v", err, vms)
	}
	db, _ := openDB()
	if err := migrate(db, 3); err != nil { // safe to repeat
		t.Fatal(err)
	}
	db.Close()

	c := vms[0].Config
	c.Graphics = GraphicsReims
	if err := SaveVM(vms[0], c); err != nil {
		t.Fatal(err)
	}
	vms, _ = ListVMs()
	if !vms[0].Config.Reims() {
		t.Fatalf("graphics must persist: %+v", vms[0].Config)
	}
	out, _ := ExportVM(vms[0])
	if !strings.Contains(string(out), `"graphics": "reims-vgpu"`) {
		t.Fatalf("export: %s", out)
	}
	f := filepath.Join(t.TempDir(), "vm.json")
	os.WriteFile(f, out, 0o644)
	if in, err := LoadConfig(f); err != nil || !in.Reims() {
		t.Fatalf("import: %+v %v", in, err)
	}
	c.GPU = "0000:03:00.0"
	if err := SaveVM(vms[0], c); err == nil {
		t.Fatal("Reims vGPU with a GPU must not be saved")
	}
}

// reimsHost is a fake host ready for Reims vGPU, with an AMD GPU.
func reimsHost(t *testing.T) (Host, *int) {
	h := fakeHost(t, "svm", "Y")
	probes := 0
	h.Output = func(name string, args ...string) ([]byte, error) {
		probes++
		return []byte(`name "apple-gfx-pci", bus PCI, desc "macOS Paravirtualized Graphics"`), nil
	}
	write(t, filepath.Join(h.SysRoot, "class/drm/card1/device/vendor"), "0x1002\n")
	write(t, h.DevUdmabuf, "")
	return h, &probes
}

func TestReimsChecks(t *testing.T) {
	h, probes := reimsHost(t)
	for _, ch := range h.ReimsChecks(16384) {
		if ch.Status != OK {
			t.Errorf("%s: %v %s", ch.Name, ch.Status, ch.Detail)
		}
	}
	h.ReimsChecks(16384)
	if *probes != 1 {
		t.Errorf("the -device help probe must be cached by path and mtime: %d runs", *probes)
	}

	// Low map count: blocking, with the sysctl command and a file to keep it.
	write(t, filepath.Join(h.ProcRoot, "sys/vm/max_map_count"), "65530\n")
	qemu, warns, err := h.ReimsReady(Config{RAMMiB: 16384})
	if err == nil || qemu != "" || !strings.Contains(err.Error(), "sudo sysctl -w vm.max_map_count=2097152") || !strings.Contains(err.Error(), "/etc/sysctl.d/") {
		t.Fatalf("low map count: %v %v", err, warns)
	}
	write(t, filepath.Join(h.ProcRoot, "sys/vm/max_map_count"), "1114112\n")

	// AMD GPU without udmabuf access: a warning naming the group, not an error.
	os.Chmod(h.DevUdmabuf, 0)
	write(t, filepath.Join(h.EtcRoot, "group"), "root:x:0:\nkvm:x:"+strconv.Itoa(os.Getgid())+":nilton\n")
	qemu, warns, err = h.ReimsReady(Config{RAMMiB: 16384})
	if os.Geteuid() != 0 && (err != nil || qemu == "" || len(warns) != 1 || !strings.Contains(warns[0], "usermod -aG kvm")) {
		t.Fatalf("udmabuf: %q %v %v", qemu, warns, err)
	}

	// No Vulkan driver.
	os.RemoveAll(filepath.Join(h.ShareRoot, "vulkan"))
	if _, _, err := h.ReimsReady(Config{RAMMiB: 8192}); err == nil || !strings.Contains(err.Error(), "Vulkan") {
		t.Fatalf("no Vulkan: %v", err)
	}

	// The checks show in the Host tab as warnings while the VM is not on Reims vGPU.
	if m := byName(h.Checks("sequoia")); m["Vulkan driver"].Status != Warn {
		t.Errorf("not relevant: %+v", m["Vulkan driver"])
	}
	if m := byName(h.ChecksFor(Config{Version: "sequoia", RAMMiB: 8192, Graphics: GraphicsReims})); m["Vulkan driver"].Status != Fail {
		t.Errorf("relevant: %+v", m["Vulkan driver"])
	}

	// A build without the device names the binary.
	other := filepath.Join(t.TempDir(), "qemu-system-x86_64")
	write(t, other, "")
	t.Setenv("MKG_REIMS_QEMU", other)
	h.Output = func(string, ...string) ([]byte, error) { return []byte(`name "vmware-svga", bus PCI`), nil }
	if c := h.checkReimsQEMU(); c.Status != Fail || !strings.Contains(c.Detail, other) {
		t.Fatalf("no apple-gfx-pci: %+v", c)
	}
}

func TestStartReimsWithoutBuild(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	t.Setenv("XDG_DATA_HOME", t.TempDir())
	t.Setenv("MKG_REIMS_QEMU", "")
	if err := SetWorkspaceRoot(t.TempDir()); err != nil {
		t.Fatal(err)
	}
	w := readyWorkspace(t)
	write(t, w.Path(DiskFile), "")
	c := Defaults()
	c.Workspace, c.Graphics, c.SSHPort = w.Dir, GraphicsReims, 2399
	err := (&VM{}).Start(c)
	if err == nil || !strings.Contains(err.Error(), "not installed") || !strings.Contains(err.Error(), "looked for") {
		t.Fatalf("missing build: %v", err)
	}
}

func TestReimsQEMUSearch(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	data := t.TempDir()
	t.Setenv("XDG_DATA_HOME", data)
	t.Setenv("MKG_REIMS_QEMU", "")
	root := t.TempDir()
	if err := SetWorkspaceRoot(root); err != nil {
		t.Fatal(err)
	}
	if _, err := ReimsQEMU(); err == nil || !strings.Contains(err.Error(), filepath.Join(root, ".reims")) {
		t.Fatalf("not installed: %v", err)
	}
	upstream := filepath.Join(data, "reims-ctl/qemu/current/bin/qemu-system-x86_64")
	write(t, upstream, "")
	if p, err := ReimsQEMU(); err != nil || p != upstream {
		t.Fatalf("upstream install: %q %v", p, err)
	}
	own := filepath.Join(root, ".reims/qemu/current/bin/qemu-system-x86_64")
	write(t, own, "")
	if p, _ := ReimsQEMU(); p != own {
		t.Fatalf("the app's install wins over upstream's: %q", p)
	}
	t.Setenv("MKG_REIMS_QEMU", upstream)
	if p, _ := ReimsQEMU(); p != upstream {
		t.Fatalf("MKG_REIMS_QEMU wins: %q", p)
	}
}

func TestReimsEnvAndExit(t *testing.T) {
	env := reimsEnv([]string{"HOME=/h", "REIMS_VGPU_WINDOW=0", "SDL_VIDEO_DRIVER=wayland", "PATH=/bin"}, false)
	if strings.Join(env, " ") != "HOME=/h SDL_VIDEO_DRIVER=wayland PATH=/bin" {
		t.Errorf("env: %v", env)
	}
	// Embedding: the window must be on X11, whatever the user's SDL driver.
	env = reimsEnv([]string{"HOME=/h", "REIMS_VGPU_WINDOW=0", "SDL_VIDEO_DRIVER=wayland"}, true)
	if strings.Join(env, " ") != "HOME=/h SDL_VIDEO_DRIVER=x11" {
		t.Errorf("embed env: %v", env)
	}
	m := explainExit(errors.New("exit status 1"), []string{"reims-vgpu: fatal: cannot open /dev/udmabuf: Permission denied"})
	if !strings.Contains(m, "usermod -aG") {
		t.Errorf("udmabuf exit: %q", m)
	}
}

// TestReimsRealizesPaused runs the Reims QEMU (MKG_TEST_REIMS_QEMU) on the Reims command,
// paused: it must start without a PCI address or audio error.
func TestReimsRealizesPaused(t *testing.T) {
	qemu := os.Getenv("MKG_TEST_REIMS_QEMU")
	if qemu == "" {
		t.Skip("MKG_TEST_REIMS_QEMU is not set")
	}
	if f, err := os.OpenFile("/dev/kvm", os.O_RDWR, 0); err != nil {
		t.Skip("no KVM: ", err)
	} else {
		f.Close()
	}
	img, err := exec.LookPath("qemu-img")
	if err != nil {
		t.Skip("qemu-img not found")
	}
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	withAudio(t, "none")
	w := NewWorkspace(t.TempDir())
	c := Defaults()
	c.Workspace, c.Graphics, c.RAMMiB, c.SSHPort = w.Dir, GraphicsReims, 4096, 2398
	c.Extras = &ExtraKexts{}
	if err := w.PrepareBoot(c, false); err != nil {
		t.Fatal(err)
	}
	if out, err := exec.Command(img, "create", "-f", "qcow2", w.Path(DiskFile), "1G").CombinedOutput(); err != nil {
		t.Fatalf("%v: %s", err, out)
	}
	run, _ := os.MkdirTemp(os.Getenv("XDG_RUNTIME_DIR"), "macos-kvm-gui-")
	args := append(BuildArgs(c, w, filepath.Join(run, "qmp.sock"), false, nil), "-S")
	cmd := exec.Command(qemu, args...)
	cmd.Dir = w.Dir
	cmd.Env = append(reimsEnv(os.Environ(), false), "REIMS_VGPU_WINDOW=0") // no window in tests
	var out strings.Builder
	cmd.Stdout, cmd.Stderr = &out, &out
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()
	select {
	case err := <-done:
		t.Fatalf("QEMU exited: %v\n%s", err, out.String())
	case <-time.After(4 * time.Second):
	}
	q, err := DialQMP(filepath.Join(run, "qmp.sock"), nil, nil)
	if err != nil {
		cmd.Process.Kill()
		t.Fatalf("QMP: %v\n%s", err, out.String())
	}
	q.Execute("quit")
	q.Close()
	select {
	case <-done:
	case <-time.After(10 * time.Second):
		cmd.Process.Kill()
		<-done
	}
	if s := out.String(); strings.Contains(s, "PCI") || strings.Contains(s, "audio") || strings.Contains(s, "Error") {
		t.Fatalf("QEMU complained:\n%s", s)
	}
	t.Logf("QEMU output: %s", out.String())
}

func TestReimsQEMURelative(t *testing.T) {
	cfg := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", cfg)
	dir := filepath.Join(cfg, "macos-kvm-gui")
	write(t, filepath.Join(dir, "bin", "qemu"), "")
	write(t, filepath.Join(dir, "settings.yaml"), "reims_qemu: bin/qemu\n")
	if q, err := ReimsQEMU(); err != nil || q != filepath.Join(dir, "bin", "qemu") {
		t.Fatalf("settings.yaml: %q %v", q, err)
	}
	t.Chdir(dir)
	t.Setenv("MKG_REIMS_QEMU", "bin/qemu")
	if q, err := ReimsQEMU(); err != nil || q != filepath.Join(dir, "bin", "qemu") {
		t.Fatalf("env: %q %v", q, err)
	}
}

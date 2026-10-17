// Package mcp is the app's Model Context Protocol server: AI agents see a VM's screen, type and
// click in it, start and stop it and install macOS from the offline installer. It runs over
// stdio only and drives VMs through the same sockets the app uses (see core.RunInfo).
package mcp

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"image/png"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

// Serve runs the server on stdin and stdout until the client disconnects. Nothing else may
// write to stdout.
func Serve(ctx context.Context, version string) error {
	return newServer(version).Run(ctx, &mcp.StdioTransport{})
}

func newServer(version string) *mcp.Server {
	s := mcp.NewServer(&mcp.Implementation{Name: "macos-kvm-gui", Version: version}, nil)
	yes, no := true, false
	destructive := &mcp.ToolAnnotations{DestructiveHint: &yes}
	readOnly := &mcp.ToolAnnotations{ReadOnlyHint: true}

	mcp.AddTool(s, &mcp.Tool{Name: "vm_list", Annotations: readOnly,
		Description: "List the VMs and whether each is running."}, vmList)
	mcp.AddTool(s, &mcp.Tool{Name: "vm_status", Annotations: readOnly,
		Description: "Show one VM: QEMU's state, whether the offline installer disc is attached, the keyboard layout macOS's recovery uses, and the display mode. " +
			"guest_phase is always unknown: look at screen_capture to see what macOS shows."}, vmStatus)
	mcp.AddTool(s, &mcp.Tool{Name: "vm_start",
		Description: "Start a VM with its screen available to these tools. It keeps running when this server exits. " +
			"Do not send keys while it boots (keys during boot change how macOS boots): wait until screen_capture shows macOS."}, vmStart)
	mcp.AddTool(s, &mcp.Tool{Name: "vm_shutdown",
		Description: "Press the VM's power button (ACPI): macOS shuts down by itself, or asks first. Setup Assistant and the recovery may ignore it: then shut down from macOS's menu, or ask the user about vm_force_stop."}, vmShutdown)
	mcp.AddTool(s, &mcp.Tool{Name: "vm_force_stop", Annotations: destructive,
		Description: "Stop a VM at once, like pulling the power cord: unsaved data in macOS is lost. Needs confirm: true; ask the user first."}, vmForceStop)
	mcp.AddTool(s, &mcp.Tool{Name: "screen_capture", Annotations: readOnly,
		Description: "The VM's screen as a PNG image. Coordinates for pointer are the guest's, given in the text with the image's scale."}, screenCapture)
	mcp.AddTool(s, &mcp.Tool{Name: "send_keys",
		Description: "Press key chords in order, for example [\"ctrl+f2\", \"right\", \"down\"] (the menu bar from the keyboard), [\"return\"], [\"cmd+q\"]. " +
			"Names are U.S. key positions: letters, digits, f1-f12, return, esc, tab, space, backspace, delete, arrows, kp_divide and other kp_ keys; modifiers shift, ctrl, alt (option), cmd."}, sendKeys)
	mcp.AddTool(s, &mcp.Tool{Name: "type_text",
		Description: "Type text as physical keys in the VM's keyboard layout (U.S. or Brazilian ABNT2). It never presses Return: send it with send_keys. " +
			"Characters the layout cannot type are refused before any key is sent."}, typeText)
	mcp.AddTool(s, &mcp.Tool{Name: "pointer",
		Description: "Move the pointer or click at guest coordinates: action move, click, double_click, right_click, scroll_up or scroll_down."}, pointer)
	mcp.AddTool(s, &mcp.Tool{Name: "installer_list", Annotations: readOnly,
		Description: "List the cached full installers (the offline install media), one per macOS build."}, installerList)
	mcp.AddTool(s, &mcp.Tool{Name: "installer_download",
		Description: "Download Apple's full installer for a macOS version (Big Sur and later, 12-18 GB) into the shared cache, check it and write the offline disc. " +
			"It resumes an interrupted download. A VM gets the disc when it starts with its install media, so download before vm_start."}, installerDownload)
	mcp.AddTool(s, &mcp.Tool{Name: "installer_remove", Annotations: destructive,
		Description: "Delete a cached full installer. Needs confirm: true; ask the user first."}, installerRemove)
	mcp.AddTool(s, &mcp.Tool{Name: "install_offline", Annotations: destructive,
		Description: "Open the recovery's Terminal and type the line that installs macOS from the offline disc, as the app's Type it in the VM does. " +
			"Call it only when screen_capture shows the recovery's window. Without unattended, erase the disk in Disk Utility first and continue in the installer. " +
			"unattended: true installs with no questions: it ERASES the VM's empty disk (or uses \"Macintosh HD\") and ACCEPTS APPLE'S macOS LICENSE for the user, " +
			"so it needs erase_disk: true and accept_license: true, which only the user can decide."}, installOffline)
	mcp.AddTool(s, &mcp.Tool{Name: "host_check", Annotations: readOnly,
		Description: "Check that this computer can run macOS VMs (KVM, QEMU, CPU), like the app's Host tab."}, hostCheck)
	mcp.AddTool(s, &mcp.Tool{Name: "vm_config_get", Annotations: readOnly,
		Description: "Read a VM's settings: version, RAM, cores, vCPUs, SSH port, graphics, resolution, display, GPU, root-patch mode and network adapters."}, vmConfigGet)
	mcp.AddTool(s, &mcp.Tool{Name: "vm_config_set", Annotations: &mcp.ToolAnnotations{DestructiveHint: &no},
		Description: "Change a stopped VM's settings with the CLI's config set keys and validation: ssh-port, ram (MiB), cores, vcpus, graphics, resolution, display, gpu, version. " +
			"root-patch and workspace are for the app or `config set` only. Returns the new settings and warnings (for example an SSH port another VM uses)."}, vmConfigSet)
	mcp.AddTool(s, &mcp.Tool{Name: "vm_create", Annotations: &mcp.ToolAnnotations{DestructiveHint: &no},
		Description: "Create a VM like `vm create`: CPU and RAM recommended for this host, a fresh network adapter, the lowest free SSH port from 2222 (or ssh_port, unless another VM has it), " +
			"its folder with the boot files and an 80 GiB disk, and select it. vm_start can start it right away; it downloads nothing (installers: installer_download)."}, vmCreate)
	mcp.AddTool(s, &mcp.Tool{Name: "vm_ssh", Annotations: destructive,
		Description: "Run one command in a running VM's macOS over SSH (127.0.0.1 and the VM's forwarded port) with the system's ssh, and return stdout, stderr and the exit code. " +
			"It logs in with the user's SSH keys or agent, or with password (never logged or stored). Commands can change or delete data in the guest: it needs confirm: true; ask the user first."}, vmSSH)
	return s
}

// --- VMs -------------------------------------------------------------------------------------

type vmArg struct {
	VM string `json:"vm,omitempty" jsonschema:"VM name or id; default: the VM selected in the app"`
}

type vmInfo struct {
	ID      string `json:"id"`
	Name    string `json:"name"`
	Version string `json:"version"`
	Running bool   `json:"running"`
}

type vmListOut struct {
	VMs []vmInfo `json:"vms"`
}

func vmList(_ context.Context, _ *mcp.CallToolRequest, _ struct{}) (*mcp.CallToolResult, vmListOut, error) {
	vms, err := core.ListVMs()
	if err != nil {
		return nil, vmListOut{}, err
	}
	out := vmListOut{VMs: []vmInfo{}}
	for _, e := range vms {
		_, running := core.FindRunning(e.Config)
		out.VMs = append(out.VMs, vmInfo{e.ID, e.DisplayName(), e.Config.Version, running})
	}
	return nil, out, nil
}

func findVM(key string) (core.VMEntry, error) {
	vms, err := core.ListVMs()
	if err != nil {
		return core.VMEntry{}, err
	}
	if key == "" {
		if e, ok := core.SelectedVM(vms); ok {
			return e, nil
		}
		return core.VMEntry{}, errors.New("no VM selected: pass vm (see vm_list)")
	}
	if e, ok := core.FindVM(vms, key); ok {
		return e, nil
	}
	return core.VMEntry{}, fmt.Errorf("no VM named %q (see vm_list)", key)
}

// running finds a VM's QEMU. Its sockets are used only inside this user's private run directory.
func running(key string) (core.VMEntry, core.RunInfo, error) {
	e, err := findVM(key)
	if err != nil {
		return e, core.RunInfo{}, err
	}
	r, ok := core.FindRunning(e.Config)
	if !ok {
		return e, r, fmt.Errorf("%s is not running (vm_start starts it)", e.DisplayName())
	}
	return e, r, nil
}

func control(r core.RunInfo) (*core.QMP, error) {
	if r.Control == "" {
		return nil, errors.New("this VM was started without a control socket (by an older version or outside the app): use the app")
	}
	if err := core.CheckRunDir(r.Control); err != nil {
		return nil, err
	}
	return core.DialQMP(r.Control, nil, nil)
}

type vmStatusOut struct {
	vmInfo
	QEMUState  string `json:"qemu_state"`
	GuestPhase string `json:"guest_phase"`
	Disc       bool   `json:"offline_disc_attached"`
	Layout     string `json:"keyboard_layout"`
	Screen     bool   `json:"screen_available"`
	Uptime     string `json:"uptime,omitempty"`
	SSHPort    int    `json:"ssh_port"`
	Warning    string `json:"warning,omitempty" jsonschema:"set when another VM uses the same SSH port"`
}

func vmStatus(_ context.Context, _ *mcp.CallToolRequest, in vmArg) (*mcp.CallToolResult, vmStatusOut, error) {
	e, err := findVM(in.VM)
	if err != nil {
		return nil, vmStatusOut{}, err
	}
	out := vmStatusOut{vmInfo: vmInfo{e.ID, e.DisplayName(), e.Config.Version, false}, QEMUState: "stopped",
		GuestPhase: "unknown", Layout: core.RecoveryKeyboard(), SSHPort: e.Config.SSHPort,
		Warning: core.SSHPortWarning(e.Config.SSHPort, e.ID)}
	r, ok := core.FindRunning(e.Config)
	if !ok {
		return nil, out, nil
	}
	out.Running, out.Disc, out.Screen, out.Uptime = true, r.Disc, r.VNCSocket != "", r.Uptime().String()
	out.QEMUState = "running"
	if q, err := control(r); err == nil {
		defer q.Close()
		if raw, err := q.Query("query-status"); err == nil {
			var st struct {
				Status string `json:"status"`
			}
			if json.Unmarshal(raw, &st) == nil && st.Status != "" {
				out.QEMUState = st.Status
			}
		}
	}
	return nil, out, nil
}

type vmStartIn struct {
	VM          string `json:"vm,omitempty" jsonschema:"VM name or id; default: the VM selected in the app"`
	VerboseBoot bool   `json:"verbose_boot,omitempty" jsonschema:"show macOS's boot log instead of the Apple logo, for this start only"`
}

// vmStart runs "macos-kvm-gui vm start" in its own session: that process owns QEMU, so the VM
// outlives this server and the agent.
func vmStart(ctx context.Context, _ *mcp.CallToolRequest, in vmStartIn) (*mcp.CallToolResult, any, error) {
	e, err := findVM(in.VM)
	if err != nil {
		return nil, nil, err
	}
	if _, ok := core.FindRunning(e.Config); ok {
		return text(e.DisplayName() + " is already running"), nil, nil
	}
	dir, err := e.Config.WorkspaceDir()
	if err != nil {
		return nil, nil, err
	}
	self, err := os.Executable()
	if err != nil {
		return nil, nil, err
	}
	// Downloads happen here, under the client's request, so the wait below covers only QEMU.
	ks, err := core.LoadExtraKexts()
	if err != nil {
		return nil, nil, err
	}
	if err := core.DownloadExtraKexts(ctx, ks); err != nil {
		return nil, nil, err
	}
	if e.Config.ReimsKext() {
		if err := core.DownloadReimsKext(ctx); err != nil {
			return nil, nil, err
		}
	}
	snapshot, err := json.Marshal(ks)
	if err != nil {
		return nil, nil, err
	}
	logPath := filepath.Join(dir, "macos-kvm-gui", "mcp-start.log")
	log, err := os.Create(logPath)
	if err != nil {
		return nil, nil, err
	}
	defer log.Close()
	args := []string{"vm", "start", "--vm", e.ID, "--display", core.DisplayApp, "--kexts-snapshot", string(snapshot)}
	if in.VerboseBoot {
		args = append(args, "--verbose-boot")
	}
	cmd := exec.Command(self, args...)
	cmd.Stdout, cmd.Stderr = log, log
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
	if err := cmd.Start(); err != nil {
		return nil, nil, err
	}
	exited := make(chan error, 1)
	go func() { exited <- cmd.Wait() }()
	for deadline := time.Now().Add(30 * time.Second); time.Now().Before(deadline); {
		select {
		case <-exited:
			b, _ := os.ReadFile(logPath)
			return nil, nil, fmt.Errorf("the VM did not start: %s", strings.TrimSpace(string(b)))
		case <-ctx.Done():
			return nil, nil, ctx.Err()
		case <-time.After(500 * time.Millisecond):
		}
		if r, ok := core.FindRunning(e.Config); ok && r.VNCSocket != "" {
			return text(fmt.Sprintf("%s started (QEMU pid %d). It boots for a minute or two: check screen_capture before sending keys.", e.DisplayName(), r.PID)), nil, nil
		}
	}
	return nil, nil, errors.New("the VM did not show up within 30 s; see " + logPath)
}

func vmShutdown(_ context.Context, _ *mcp.CallToolRequest, in vmArg) (*mcp.CallToolResult, any, error) {
	e, r, err := running(in.VM)
	if err != nil {
		return nil, nil, err
	}
	q, err := control(r)
	if err != nil {
		return nil, nil, err
	}
	defer q.Close()
	if err := q.Execute("system_powerdown"); err != nil {
		return nil, nil, err
	}
	return text("Pressed the power button of " + e.DisplayName() + ": macOS shuts down by itself, or asks on its screen."), nil, nil
}

type confirmVM struct {
	VM      string `json:"vm,omitempty" jsonschema:"VM name or id; default: the VM selected in the app"`
	Confirm bool   `json:"confirm,omitempty" jsonschema:"must be true: the user agreed to lose unsaved data in macOS"`
}

func vmForceStop(_ context.Context, _ *mcp.CallToolRequest, in confirmVM) (*mcp.CallToolResult, any, error) {
	if !in.Confirm {
		return nil, nil, errors.New("a forced stop loses unsaved data in macOS: ask the user, then call it with confirm: true")
	}
	e, r, err := running(in.VM)
	if err != nil {
		return nil, nil, err
	}
	q, err := control(r)
	if err != nil {
		return nil, nil, err
	}
	defer q.Close()
	q.Execute("quit") // QEMU exits before it replies
	return text(e.DisplayName() + " was stopped."), nil, nil
}

// --- screen and input ------------------------------------------------------------------------

// inputMu serializes input per screen, so two calls never interleave their keys.
var (
	inputMu   sync.Mutex
	inputLock = map[string]*sync.Mutex{}
)

// screenConn is one connection to a VM's screen, for one tool call.
type screenConn struct {
	v       *core.VNC
	updates chan struct{}
	runErr  chan error
}

// withScreen connects to a running VM's screen and runs f with the screen's lock held, so calls
// never interleave keys, and a capture never disconnects (which makes QEMU release every held
// key) in the middle of a chord. A cancelled call stops before its next key.
func withScreen(ctx context.Context, key string, f func(e core.VMEntry, sc screenConn) error) error {
	e, r, err := running(key)
	if err != nil {
		return err
	}
	if r.VNCSocket == "" {
		return errors.New(e.DisplayName() + " shows its screen in QEMU's own window: screen and input need Display: In the app (Settings › Display)")
	}
	if err := core.CheckRunDir(r.VNCSocket); err != nil {
		return err
	}
	inputMu.Lock()
	m := inputLock[r.VNCSocket]
	if m == nil {
		m = &sync.Mutex{}
		inputLock[r.VNCSocket] = m
	}
	inputMu.Unlock()
	m.Lock()
	defer m.Unlock()
	if err := ctx.Err(); err != nil { // cancelled while waiting for another call
		return err
	}
	v, err := core.DialVNC(r.VNCSocket)
	if err != nil {
		return err
	}
	defer v.Close()
	sc := screenConn{v: v, updates: make(chan struct{}, 64), runErr: make(chan error, 1)}
	v.OnUpdate = func() {
		select {
		case sc.updates <- struct{}{}:
		default:
		}
	}
	go func() { sc.runErr <- v.Run() }()                     // ends when v is closed
	if err := sleep(ctx, 300*time.Millisecond); err != nil { // QEMU says it takes extended key events
		return err
	}
	return f(e, sc)
}

func sleep(ctx context.Context, d time.Duration) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-time.After(d):
		return nil
	}
}

type captureIn struct {
	VM       string `json:"vm,omitempty" jsonschema:"VM name or id; default: the VM selected in the app"`
	MaxWidth int    `json:"max_width,omitempty" jsonschema:"scale the image down to at most this width (default 1280; 0 or more than the screen keeps it)"`
}

func screenCapture(ctx context.Context, _ *mcp.CallToolRequest, in captureIn) (*mcp.CallToolResult, any, error) {
	var frame *image.RGBA
	var name string
	err := withScreen(ctx, in.VM, func(e core.VMEntry, sc screenConn) error {
		var err error
		name = e.DisplayName()
		frame, err = sc.v.Snapshot(sc.updates, sc.runErr, 10*time.Second)
		return err
	})
	if err != nil {
		return nil, nil, err
	}
	if in.MaxWidth == 0 {
		in.MaxWidth = 1280
	}
	img, scale := scaleDown(frame, in.MaxWidth)
	var buf bytes.Buffer
	if err := png.Encode(&buf, img); err != nil {
		return nil, nil, err
	}
	w, h := frame.Bounds().Dx(), frame.Bounds().Dy()
	note := fmt.Sprintf("%s screen, %dx%d guest pixels", name, w, h)
	if scale != 1 {
		note += fmt.Sprintf("; image scaled by %.3f: divide image coordinates by it for pointer", scale)
	}
	return &mcp.CallToolResult{Content: []mcp.Content{
		&mcp.ImageContent{Data: buf.Bytes(), MIMEType: "image/png"},
		&mcp.TextContent{Text: note},
	}}, nil, nil
}

// scaleDown shrinks src to at most maxW wide (nearest pixel, like the app's preview).
func scaleDown(src *image.RGBA, maxW int) (image.Image, float64) {
	w, h := src.Bounds().Dx(), src.Bounds().Dy()
	if maxW <= 0 || w <= maxW {
		return src, 1
	}
	nh := h * maxW / w
	dst := image.NewRGBA(image.Rect(0, 0, maxW, nh))
	for y := range nh {
		sy := y * h / nh
		for x := range maxW {
			so := src.PixOffset(x*w/maxW, sy)
			copy(dst.Pix[dst.PixOffset(x, y):], src.Pix[so:so+4])
		}
	}
	return dst, float64(maxW) / float64(w)
}

type keysIn struct {
	VM   string   `json:"vm,omitempty" jsonschema:"VM name or id; default: the VM selected in the app"`
	Keys []string `json:"keys" jsonschema:"chords to press in order, such as ctrl+f2, return or cmd+q"`
}

func sendKeys(ctx context.Context, _ *mcp.CallToolRequest, in keysIn) (*mcp.CallToolResult, any, error) {
	var chords [][]core.KeyCode
	for _, k := range in.Keys {
		c, err := core.ParseChord(k)
		if err != nil {
			return nil, nil, err
		}
		chords = append(chords, c)
	}
	if len(chords) == 0 {
		return nil, nil, errors.New("keys is empty")
	}
	err := withScreen(ctx, in.VM, func(_ core.VMEntry, sc screenConn) error { return pressAll(ctx, sc.v, chords, 80*time.Millisecond) })
	if err != nil {
		return nil, nil, err
	}
	return text(fmt.Sprintf("Pressed %s.", strings.Join(in.Keys, ", "))), nil, nil
}

// pressAll presses chords in order; a cancel stops it between chords (each chord is pressed and
// released whole, so no key stays down).
func pressAll(ctx context.Context, v *core.VNC, chords [][]core.KeyCode, gap time.Duration) error {
	for _, c := range chords {
		if err := ctx.Err(); err != nil {
			return err
		}
		if err := v.Chord(c); err != nil {
			return err
		}
		if err := sleep(ctx, gap); err != nil {
			return err
		}
	}
	return nil
}

type typeIn struct {
	VM     string `json:"vm,omitempty" jsonschema:"VM name or id; default: the VM selected in the app"`
	Text   string `json:"text" jsonschema:"the text; no line breaks"`
	Layout string `json:"layout,omitempty" jsonschema:"us or br (Brazilian ABNT2); default: the layout the recovery uses on this host"`
}

func typeText(ctx context.Context, _ *mcp.CallToolRequest, in typeIn) (*mcp.CallToolResult, any, error) {
	layout := core.GuestLayout()
	switch in.Layout {
	case "":
	case "us":
		layout = 0
	case "br":
		layout = 128
	default:
		return nil, nil, fmt.Errorf("layout %q: use us or br", in.Layout)
	}
	chords, err := core.TypeKeys(in.Text, layout)
	if err != nil {
		return nil, nil, err
	}
	if err := withScreen(ctx, in.VM, func(_ core.VMEntry, sc screenConn) error { return pressAll(ctx, sc.v, chords, 0) }); err != nil {
		return nil, nil, err
	}
	return text(fmt.Sprintf("Typed %d characters (Return not pressed).", len([]rune(in.Text)))), nil, nil
}

type pointerIn struct {
	VM     string `json:"vm,omitempty" jsonschema:"VM name or id; default: the VM selected in the app"`
	Action string `json:"action" jsonschema:"move, click, double_click, right_click, scroll_up or scroll_down"`
	X      int    `json:"x" jsonschema:"guest x coordinate"`
	Y      int    `json:"y" jsonschema:"guest y coordinate"`
}

func pointer(ctx context.Context, _ *mcp.CallToolRequest, in pointerIn) (*mcp.CallToolResult, any, error) {
	press := map[string][]uint8{
		"move": nil, "click": {core.ButtonLeft}, "double_click": {core.ButtonLeft, core.ButtonLeft},
		"right_click": {core.ButtonRight}, "scroll_up": {core.ButtonWheelUp}, "scroll_down": {core.ButtonWheelDown},
	}
	buttons, ok := press[in.Action]
	if !ok {
		return nil, nil, fmt.Errorf("action %q: use move, click, double_click, right_click, scroll_up or scroll_down", in.Action)
	}
	err := withScreen(ctx, in.VM, func(_ core.VMEntry, sc screenConn) error {
		v := sc.v
		w, h := v.Size()
		if in.X < 0 || in.Y < 0 || in.X >= w || in.Y >= h {
			return fmt.Errorf("(%d, %d) is outside the %dx%d screen", in.X, in.Y, w, h)
		}
		if err := v.Pointer(0, in.X, in.Y); err != nil {
			return err
		}
		for _, b := range buttons {
			time.Sleep(60 * time.Millisecond)
			if err := v.Pointer(b, in.X, in.Y); err != nil {
				return err
			}
			time.Sleep(60 * time.Millisecond)
			if err := v.Pointer(0, in.X, in.Y); err != nil {
				return err
			}
		}
		return nil
	})
	if err != nil {
		return nil, nil, err
	}
	return text(fmt.Sprintf("%s at (%d, %d).", in.Action, in.X, in.Y)), nil, nil
}

// --- offline installer -----------------------------------------------------------------------

type installerInfo struct {
	Version string  `json:"version"`
	Build   string  `json:"build"`
	SizeGB  float64 `json:"size_gb"`
	Path    string  `json:"path"`
}

type installerListOut struct {
	Installers []installerInfo `json:"installers"`
}

func installerList(_ context.Context, _ *mcp.CallToolRequest, _ struct{}) (*mcp.CallToolResult, installerListOut, error) {
	out := installerListOut{Installers: []installerInfo{}}
	for _, c := range core.CachedInstallers() {
		out.Installers = append(out.Installers, installerInfo{c.Version, c.Build, float64(c.Size) / 1e9, c.Path})
	}
	return nil, out, nil
}

type downloadIn struct {
	Version string `json:"version" jsonschema:"macOS version: big-sur, monterey, ventura, sonoma, sequoia or tahoe"`
}

// overall maps the download's stages to one value that only grows (0-100).
func overall(p core.Progress) float64 {
	pct := max(p.Pct, 0)
	switch {
	case strings.HasPrefix(p.Stage, "Downloading"):
		return 1 + pct*0.89
	case strings.HasPrefix(p.Stage, "Checking"):
		return 90 + pct*0.09
	case strings.HasPrefix(p.Stage, "Writing"):
		return 99
	}
	return 0
}

func installerDownload(ctx context.Context, req *mcp.CallToolRequest, in downloadIn) (*mcp.CallToolResult, any, error) {
	if !core.InstallerSupported(in.Version) {
		return nil, nil, fmt.Errorf("Apple has a full installer only for Big Sur and later, not %q", in.Version)
	}
	token := req.Params.GetProgressToken()
	var mu sync.Mutex
	last, lastSent := -1.0, time.Time{}
	ci, err := core.DownloadInstaller(ctx, in.Version, func(p core.Progress) {
		if token == nil {
			return
		}
		mu.Lock()
		defer mu.Unlock()
		v := overall(p)
		if v <= last || (time.Since(lastSent) < time.Second && v < 99) {
			return
		}
		last, lastSent = v, time.Now()
		req.Session.NotifyProgress(ctx, &mcp.ProgressNotificationParams{ProgressToken: token, Progress: v, Total: 100, Message: p.Stage})
	}, func(s string) { fmt.Fprintln(os.Stderr, s) })
	if err != nil {
		return nil, nil, err
	}
	return text(fmt.Sprintf("Offline installer ready: macOS %s (%s), %.1f GB, %s. VMs of this version get it when they start with their install media.",
		ci.Version, ci.Build, float64(ci.Size)/1e9, ci.Path)), nil, nil
}

type removeIn struct {
	Build   string `json:"build" jsonschema:"the build, from installer_list"`
	Confirm bool   `json:"confirm,omitempty" jsonschema:"must be true: the user agreed to delete it"`
}

func installerRemove(_ context.Context, _ *mcp.CallToolRequest, in removeIn) (*mcp.CallToolResult, any, error) {
	if !in.Confirm {
		return nil, nil, errors.New("deleting an installer means downloading 12-18 GB again: ask the user, then call it with confirm: true")
	}
	if err := core.RemoveInstaller(in.Build); err != nil {
		return nil, nil, err
	}
	return text("Deleted the installer " + in.Build + "."), nil, nil
}

type installIn struct {
	VM            string `json:"vm,omitempty" jsonschema:"VM name or id; default: the VM selected in the app"`
	Unattended    bool   `json:"unattended,omitempty" jsonschema:"install with no questions; needs erase_disk and accept_license"`
	EraseDisk     bool   `json:"erase_disk,omitempty" jsonschema:"the user agreed that the VM's empty disk is erased (or Macintosh HD is installed over)"`
	AcceptLicense bool   `json:"accept_license,omitempty" jsonschema:"the user accepts Apple's macOS software license agreement"`
}

func installOffline(ctx context.Context, _ *mcp.CallToolRequest, in installIn) (*mcp.CallToolResult, any, error) {
	if in.Unattended && !in.AcceptLicense {
		return nil, nil, errors.New("the unattended install accepts Apple's macOS license for the user: ask them, then pass accept_license: true")
	}
	if in.Unattended && !in.EraseDisk {
		return nil, nil, errors.New("the unattended install erases the VM's empty disk: ask the user, then pass erase_disk: true")
	}
	if !core.GuestTypingOK() {
		return nil, nil, errors.New("this keyboard layout moves the letters of the line (Dvorak, Russian): type it in the recovery's Terminal yourself: " + core.DiscCommand)
	}
	e, r, err := running(in.VM)
	if err != nil {
		return nil, nil, err
	}
	if !r.Disc {
		return nil, nil, errors.New(e.DisplayName() + " has no offline disc: download the installer (installer_download), then start the VM with its install media")
	}
	line := core.DiscTyped
	if in.Unattended {
		line += " -u"
	}
	chords, err := core.GuestKeys(line)
	if err != nil {
		return nil, nil, err
	}
	err = withScreen(ctx, in.VM, func(_ core.VMEntry, sc screenConn) error {
		if err := pressAll(ctx, sc.v, core.RecoveryTerminal(), 120*time.Millisecond); err != nil {
			return err
		}
		if err := sleep(ctx, 2*time.Second); err != nil { // the Terminal window opens
			return err
		}
		return pressAll(ctx, sc.v, chords, 0)
	})
	if err != nil {
		return nil, nil, err
	}
	msg := "Typed the install line in the recovery's Terminal. It copies the installer from the disc and opens it: continue in the installer."
	if in.Unattended {
		msg = "Typed the unattended install line. The VM erases its empty disk, installs and restarts by itself (about 30 minutes) up to Setup Assistant; follow it with screen_capture."
	}
	return text(msg + " Check screen_capture: if the Terminal did not open, the keys went to the recovery's window."), nil, nil
}

type checkOut struct {
	Ready  bool        `json:"ready"`
	Checks []checkInfo `json:"checks"`
}

type checkInfo struct {
	Name   string `json:"name"`
	Status string `json:"status"`
	Detail string `json:"detail,omitempty"`
	Hint   string `json:"hint,omitempty"`
}

func hostCheck(_ context.Context, _ *mcp.CallToolRequest, in vmArg) (*mcp.CallToolResult, checkOut, error) {
	version := core.Defaults().Version
	if e, err := findVM(in.VM); err == nil {
		version = e.Config.Version
	}
	out := checkOut{Ready: true, Checks: []checkInfo{}}
	for _, c := range core.SystemHost().Checks(version) {
		out.Checks = append(out.Checks, checkInfo{c.Name, fmt.Sprint(c.Status), c.Detail, c.Hint})
		if c.Status == core.Fail {
			out.Ready = false
		}
	}
	return nil, out, nil
}

func text(s string) *mcp.CallToolResult {
	return &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: s}}}
}

// --- settings --------------------------------------------------------------------------------

type vmConfigOut struct {
	ID string `json:"id"`
	core.Config
	Notice   string   `json:"notice,omitempty" jsonschema:"what changed from the request, such as an SSH port another VM had"`
	Warnings []string `json:"warnings,omitempty"`
}

func vmConfigGet(_ context.Context, _ *mcp.CallToolRequest, in vmArg) (*mcp.CallToolResult, vmConfigOut, error) {
	e, err := findVM(in.VM)
	if err != nil {
		return nil, vmConfigOut{}, err
	}
	return nil, vmConfigOut{ID: e.ID, Config: e.Config, Warnings: core.ConfigWarnings(e)}, nil
}

type configSetIn struct {
	VM         string  `json:"vm,omitempty" jsonschema:"VM name or id; default: the VM selected in the app"`
	SSHPort    *int    `json:"ssh-port,omitempty" jsonschema:"SSH port on 127.0.0.1 (1024-65535)"`
	RAM        *int    `json:"ram,omitempty" jsonschema:"RAM in MiB (at least 4096)"`
	Cores      *int    `json:"cores,omitempty" jsonschema:"CPU cores: 1, 2, 4, 8 or 16"`
	VCPUs      *int    `json:"vcpus,omitempty" jsonschema:"vCPUs, a multiple of cores"`
	Graphics   *string `json:"graphics,omitempty" jsonschema:"vmware-svga or reims-vgpu"`
	Resolution *string `json:"resolution,omitempty" jsonschema:"Reims vGPU display size, such as 1920x1080"`
	Display    *string `json:"display,omitempty" jsonschema:"app or window"`
	GPU        *string `json:"gpu,omitempty" jsonschema:"PCI address of a passed-through GPU, or empty for none"`
	Version    *string `json:"version,omitempty" jsonschema:"macOS version: high-sierra ... tahoe"`
	RootPatch  *bool   `json:"root-patch,omitempty" jsonschema:"refused: use the app or config set"`
	Workspace  *string `json:"workspace,omitempty" jsonschema:"refused: use the app or config set"`
}

func vmConfigSet(_ context.Context, _ *mcp.CallToolRequest, in configSetIn) (*mcp.CallToolResult, vmConfigOut, error) {
	if in.RootPatch != nil {
		return nil, vmConfigOut{}, errors.New("root-patch: use the app or `config set` (it needs a confirmation and downloads OCLP's kexts)")
	}
	if in.Workspace != nil {
		return nil, vmConfigOut{}, errors.New("workspace: use the app or `config set` (it changes the folder; moving is vm move)")
	}
	values := map[string]string{}
	for k, v := range map[string]*int{"ssh-port": in.SSHPort, "ram": in.RAM, "cores": in.Cores, "vcpus": in.VCPUs} {
		if v != nil {
			values[k] = fmt.Sprint(*v)
		}
	}
	for k, v := range map[string]*string{"graphics": in.Graphics, "resolution": in.Resolution, "display": in.Display, "gpu": in.GPU, "version": in.Version} {
		if v != nil {
			values[k] = *v
		}
	}
	e, err := findVM(in.VM)
	if err != nil {
		return nil, vmConfigOut{}, err
	}
	e, warnings, err := core.SetVMConfig(e, values)
	if err != nil {
		return nil, vmConfigOut{}, err
	}
	return nil, vmConfigOut{ID: e.ID, Config: e.Config, Warnings: warnings}, nil
}

type createIn struct {
	Name    string `json:"name" jsonschema:"the VM's name"`
	Version string `json:"version,omitempty" jsonschema:"macOS version: high-sierra ... tahoe; default sonoma"`
	Folder  string `json:"folder,omitempty" jsonschema:"VM folder; default <Workspace>/<name>"`
	SSHPort int    `json:"ssh_port,omitempty" jsonschema:"SSH port on 127.0.0.1; default the lowest free one from 2222; another VM's port is replaced by a free one"`
}

func vmCreate(ctx context.Context, _ *mcp.CallToolRequest, in createIn) (*mcp.CallToolResult, vmConfigOut, error) {
	if in.Version == "" {
		in.Version = core.Defaults().Version
	}
	e, err := core.CreateVMWithDefaults(ctx, in.Name, in.Version, in.Folder, in.SSHPort)
	if err != nil {
		if e.ID != "" {
			return nil, vmConfigOut{}, fmt.Errorf("%s was added but its folder is not ready: %w", e.DisplayName(), err)
		}
		return nil, vmConfigOut{}, err
	}
	return nil, vmConfigOut{ID: e.ID, Config: e.Config, Notice: e.Notice}, nil
}

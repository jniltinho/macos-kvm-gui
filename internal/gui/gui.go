// Package gui is the desktop interface: a VirtualBox-style window over internal/core.
package gui

import (
	"context"
	"errors"
	"fmt"
	"image"
	"os"
	osuser "os/user"
	"strconv"
	"strings"
	"time"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/app"
	"fyne.io/fyne/v2/canvas"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/dialog"
	"fyne.io/fyne/v2/lang"
	"fyne.io/fyne/v2/theme"
	"fyne.io/fyne/v2/widget"

	"github.com/jniltinho/macos-kvm-gui/internal/assets"
	"github.com/jniltinho/macos-kvm-gui/internal/core"
	"github.com/jniltinho/macos-kvm-gui/internal/embed"
)

var logo = fyne.NewStaticResource("logo.svg", assets.Logo)

// version is shown in the window title; Run sets it.
var version = "dev"

const homeLab = "For home lab use. macOS is Apple's software: its license only allows it on Apple-branded hardware, so make sure your use complies with it."

const manualSteps = `1. In the OpenCore menu, **macOS Base System** is selected and starts by itself after five seconds (press Enter to start it now).
2. Open **Disk Utility**, enable *View › Show All Devices*, select the **QEMU HARDDISK** disk and use **Erase** (APFS, GUID).
3. Close Disk Utility and choose **Reinstall macOS**. The VM restarts a few times: in OpenCore pick **macOS Installer**, then the installed disk.
4. Once macOS boots from the disk, remove the install media here to free space.`

const videoTips = `The VM has no graphics acceleration: macOS has no driver for the emulated display or for NVIDIA GPUs, so the CPU draws the screen. To make it smoother:

1. Use **Recommended for this host** in Settings › System (more vCPUs and RAM).
2. In macOS, under *System Settings › Accessibility › Display*, turn on **Reduce motion** and **Reduce transparency**.
3. Under *Desktop & Dock*, minimize with **Scale** and turn off **Animate opening applications**.
4. Use a static wallpaper (the default Sonoma wallpaper is animated and stays blank without Metal) and keep 1920x1080.

Real acceleration needs a macOS-supported AMD GPU passed through with VFIO: see the GPU tab.`

// rootPatchSteps is what the user does inside macOS once root-patch mode is on.
const rootPatchSteps = `Root-patch mode lets OpenCore Legacy Patcher (OCLP) install the Kepler drivers Apple removed after Big Sur. Inside macOS, with the Kepler card passed through:

1. Turn **FileVault off**, and keep a backup of the VM's folder (disk, OVMF_VARS and the macos-kvm-gui folder).
2. Download OCLP from github.com/dortania/OpenCore-Legacy-Patcher and open it. **Never use "Build and Install OpenCore"**: the app's OpenCore and Mac model stay.
3. Click **Post-Install Root Patch**, then **Start Root Patching**. On Sequoia OCLP downloads MetallibSupportPkg, so macOS needs internet.
4. Restart. Metal should now use the Kepler card (System Information › Graphics).
5. **Every macOS update removes the patch**: turn automatic downloads off, update on purpose, then patch again.
6. To stop using it: in OCLP, **Revert Root Patches**, restart, then turn root-patch mode off here.

What the mode weakens: System Integrity Protection drops to 0x803 (unsigned kexts, writable system volume, unauthenticated root), and AMFIPass lets the patched drivers load. Experimental: untested with a real Kepler card on this app.`

const gpuNotes = `Gives the VM a real GPU, the only way to get Metal. The GPU's display output then goes to a monitor plugged into that card; the QEMU window stays for keyboard and mouse.

- Supported: RX 460–590, Vega 56/64, Radeon VII, RX 5500/5600/5700, RX 6600/6600 XT, RX 6800/6800 XT/6900 XT.
- NVIDIA Kepler (GT 710/730 with GK208, GTX 650–780, Quadro K with Kepler chips): native up to Big Sur; Monterey to Sequoia with **root-patch mode** below and OpenCore Legacy Patcher inside macOS.
- Not supported: RX 7000, RX 6400/6500, NVIDIA Maxwell and newer (GTX 750, 900, 10, 16, 20 series and up), Intel and AMD Ryzen iGPUs.
- The host must show its screen on another GPU (set the integrated GPU as primary display in the BIOS).
- **Reserve** writes the VFIO settings and needs a host reboot; **Release** undoes it.
- Polaris, Vega and RX 5000 cards may need a host reboot after each VM restart (AMD reset bug). Navi cards may need ` + "`agdpmod=pikera`" + ` in OpenCore's boot-args.
- Untested on real hardware: the reference host has no supported AMD card.`

type ui struct {
	win     fyne.Window
	cfgPath string
	cfg     core.Config
	host    core.Host
	vm      *core.VM
	busy    bool

	checks                         *fyne.Container
	steps                          []*itemCard // the Install tab's steps
	hostSummary                    *widget.Label
	installTitle                   *widget.Label
	installBar                     *progressLine
	installFiles                   *widget.Label
	btnStartInstall                *widget.Button
	btnVerboseInstall              *widget.Button
	btnFullInstaller               *widget.Button
	btnDeleteInstaller             *widget.Button
	installerInfo                  *widget.Label
	installMode                    *widget.Label
	watching                       bool      // the screen shows a VM another process runs (watchOutside)
	watchID                        string    // which VM it shows
	watchVNC                       *core.VNC // its connection, closed when another VM is selected
	lastWatch                      time.Time // the last attempt, so they are at most one every 3 s
	installerCmd                   *widget.Entry
	installerCmdBox                *fyne.Container
	stage                          *widget.Label
	bar                            *widget.ProgressBar
	barInf                         *widget.ProgressBarInfinite
	diskSize                       *widget.Entry
	log                            *widget.TextGrid
	logScroll                      *container.Scroll
	logLines                       []string
	gpuList                        *fyne.Container
	gpuSelect                      *widget.Select
	rootPatch                      *widget.Check
	rootPatchInfo                  *widget.Label
	rootPatchSetting               bool // SetChecked from refresh, not the user
	gpuStatus                      *widget.Label
	gpus                           []core.GPU
	reimsChecks                    *fyne.Container
	reimsSummary, reimsInfo        *widget.Label
	btnReimsInstall, btnReimsBuild *widget.Button
	header                         *widget.Label // VM state chip in the toolbar
	status                         *widget.Label // status bar
	tabs                           *container.AppTabs
	vmItem                         *widget.List
	details                        map[string]*canvas.Text
	screen                         *screen
	screenHolder                   *fyne.Container
	screenWin                      fyne.Window
	emb                            *reimsEmbed
	btnDetach, btnFull             *toolButton
	btnNew, btnEdit, btnDelete     *toolButton
	vms                            []core.VMEntry
	cur                            core.VMEntry // the selected VM
	runningID                      string       // the VM QEMU is running, if any
	singleFile                     bool         // --config: one VM file, no list management
	preview                        *canvas.Image
	previewImg                     *image.RGBA
	previewBg                      *canvas.Rectangle
	previewName                    *canvas.Text
	previewIdle                    *fyne.Container
	btnExport, btnImport           *toolButton
	menuFor                        map[*fyne.MenuItem]*toolButton // menu items that mirror a toolbar button
	lastPreview                    time.Time
	previewGen                     int // bumped when the VM shown changes or stops: older preview frames are dropped

	btnPrepare, btnDownload, btnDisk, btnRemove *widget.Button
	btnStart, btnPowerdown, btnKill             *toolButton
	btnReserve, btnRelease                      *widget.Button
}

// Run opens the main window and blocks until it closes.
func Run(appVersion, configPath string) {
	version = appVersion
	// The whole interface is in English, including Fyne's own dialogs (file pickers, buttons),
	// which otherwise follow the desktop's language. LANGUAGE only selects translations.
	os.Setenv("LANGUAGE", "en")
	preferX11()                                       // before the app starts: Fyne picks the platform at init
	lang.AddTranslationsForLocale([]byte("{}"), "en") // re-reads the language set above
	a := app.NewWithID("io.github.jniltinho.macos-kvm-gui")
	a.Settings().SetTheme(appTheme{Theme: theme.DefaultTheme(), mode: themeMode()})
	a.SetIcon(logo)
	u := &ui{win: a.NewWindow("macOS KVM " + version), host: core.SystemHost(), vm: &core.VM{}}
	u.emb = &reimsEmbed{u: u}
	err := u.loadVMs(configPath)

	u.vm.OnLog = func(s string) { fyne.Do(func() { u.appendLog(s) }) }
	u.vm.OnState = func(s core.VMState, msg string) {
		fyne.Do(func() {
			if s == core.Stopped {
				u.emb.release()
			}
			u.refresh()
			if msg != "" {
				u.appendLog("[VM] " + msg)
				if s != core.Running {
					dialog.ShowInformation("VM: "+s.String(), msg, u.win)
				}
			}
		})
	}

	top := u.toolbar()
	u.tabs = container.NewAppTabs(
		container.NewTabItemWithIcon("Details", theme.InfoIcon(), u.detailsTab()),
		container.NewTabItemWithIcon("Screen", theme.ComputerIcon(), u.screenTab()),
		container.NewTabItemWithIcon("Install", theme.DownloadIcon(), u.installTab()),
		container.NewTabItemWithIcon("Host", theme.HomeIcon(), u.hostTab()),
		container.NewTabItemWithIcon("GPU", gpuTabIcon(), u.gpuTab()),
		container.NewTabItemWithIcon("Log", theme.ListIcon(), u.logTab()),
	)
	split := container.NewHSplit(u.vmList(), u.tabs)
	split.Offset = listWidth()
	if embed.Available() { // menus that the embedded guest window can hide under (design D6)
		top = container.NewVBox(menuBar(u.win, u.mainMenu()), top)
	} else {
		u.win.SetMainMenu(u.mainMenu())
	}
	u.win.SetContent(container.NewBorder(top, u.statusBar(), nil, nil, split))
	releaseOnSignals(a, u)
	a.Settings().AddListener(func(fyne.Settings) {
		recolorThemed()
		u.refresh() // cards take their colors when they are marked
	})
	u.win.Resize(fyne.NewSize(1280, 820))
	u.win.SetCloseIntercept(func() {
		if u.vm.State() != core.Stopped {
			dialog.ShowInformation("VM running", "Shut down the VM before closing the app.", u.win)
			return
		}
		// The list keeps the width the user dragged it to, like VirtualBox.
		core.SetPreference("list-width", strconv.FormatFloat(split.Offset, 'f', 3, 64))
		u.win.Close()
	})
	u.refresh()
	if err != nil {
		dialog.ShowError(err, u.win)
	}
	// VMs that other processes start or stop (the CLI, an AI agent through the MCP server) show
	// up in the list, and the current one's screen, without a click.
	go func() {
		for range time.Tick(3 * time.Second) {
			fyne.Do(func() {
				if u.vm.State() == core.Stopped {
					u.vmItem.Refresh()
					u.watchOutside()
				}
			})
		}
	}()
	u.win.ShowAndRun()
}

// listWidth is the VM list's share of the window: the saved one, or a quarter.
func listWidth() float64 {
	if v, err := strconv.ParseFloat(core.Preference("list-width"), 64); err == nil && v >= 0.05 && v <= 0.8 {
		return v
	}
	return 0.25
}

// section is a titled card; every tab is built from these.
func section(title, subtitle string, content ...fyne.CanvasObject) fyne.CanvasObject {
	return widget.NewCard(title, subtitle, container.NewVBox(content...))
}

// markdown is wrapped rich text: unwrapped text would force the whole right pane wide.
func markdown(md string) *widget.RichText {
	r := widget.NewRichTextFromMarkdown(md)
	r.Wrapping = fyne.TextWrapWord
	return r
}

func wrapped(text string) *widget.Label {
	l := widget.NewLabel(text)
	l.Wrapping = fyne.TextWrapWord
	return l
}

// workspace uses the saved configuration; each operation gets its own copy.
func (u *ui) workspace() (core.Workspace, error) {
	dir, err := u.cfg.WorkspaceDir()
	return core.NewWorkspace(dir), err
}

func (u *ui) appendLog(s string) {
	u.logLines = append(u.logLines, s)
	if len(u.logLines) > 1000 {
		u.logLines = u.logLines[len(u.logLines)-1000:]
	}
	u.log.SetText(strings.Join(u.logLines, "\n"))
	u.logScroll.ScrollToBottom()
}

func (u *ui) logAsync(s string) { fyne.Do(func() { u.appendLog(s) }) }

// run executes fn off the UI thread, with the action buttons disabled.
func (u *ui) run(name string, fn func() error, onOK func()) {
	u.busy = true
	u.refresh()
	u.appendLog("== " + name)
	go func() {
		err := fn()
		fyne.Do(func() {
			u.busy = false
			u.bar.Hide()
			u.barInf.Stop()
			u.barInf.Hide()
			u.stage.SetText("")
			if err != nil {
				u.appendLog("error: " + err.Error())
				dialog.ShowError(fmt.Errorf("%s: %w", name, err), u.win)
			} else {
				u.appendLog("ok: " + name)
				if onOK != nil {
					onOK()
				}
			}
			u.refresh()
		})
	}()
}

// saveConfig validates, persists and applies a new configuration.
func (u *ui) saveConfig(c core.Config) bool {
	if err := core.SaveVM(u.cur, c); err != nil {
		dialog.ShowError(err, u.win)
		return false
	}
	u.cur.Config = c
	u.cfg = c
	for i := range u.vms { // the list's copy too, or selecting the VM again brings the old one back
		if u.vms[i].ID == u.cur.ID {
			u.vms[i].Config = c
		}
	}
	u.refresh()
	return true
}

func statusImportance(s core.Status) widget.Importance {
	switch s {
	case core.OK:
		return widget.SuccessImportance
	case core.Warn:
		return widget.WarningImportance
	case core.Fail:
		return widget.DangerImportance
	}
	return widget.LowImportance
}

func statusIcon(s core.Status) fyne.Resource {
	switch s {
	case core.OK:
		return theme.ConfirmIcon()
	case core.Warn:
		return theme.WarningIcon()
	case core.Fail:
		return theme.ErrorIcon()
	}
	return theme.QuestionIcon()
}

func (u *ui) hostTab() fyne.CanvasObject {
	u.checks = container.NewVBox()
	recheck := widget.NewButtonWithIcon("Check again", theme.ViewRefreshIcon(), func() { u.refreshChecks(); u.refreshReimsInfo() })
	fix := widget.NewButtonWithIcon("Set up host", theme.SettingsIcon(), func() {
		me, err := osuser.Current()
		if err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		go func() {
			msg, err := u.host.SetupHost(me.Username)
			fyne.Do(func() {
				switch {
				case errors.Is(err, core.ErrCancelled):
					dialog.ShowInformation("Set up host", "Cancelled: nothing was changed.", u.win)
				case err != nil:
					dialog.ShowError(err, u.win)
				default:
					dialog.ShowInformation("Set up host", msg, u.win)
				}
				u.refreshChecks()
			})
		}()
	})
	fix.Importance = widget.HighImportance
	setupNote := caption("Set up host asks for your password once, then installs missing QEMU packages, loads the KVM module, " +
		"adds you to the kvm group and enables ignore_msrs. The libvirt and input groups are not needed.")
	setupNote.Wrapping = fyne.TextWrapWord
	notice := wrapped(homeLab)
	notice.Importance = widget.WarningImportance
	u.hostSummary = widget.NewLabel("")
	u.hostSummary.TextStyle = fyne.TextStyle{Bold: true}
	header := tabHeader("Host checks", "system", theme.ComputerIcon(),
		container.NewBorder(nil, nil, nil, container.NewHBox(recheck, fix), u.hostSummary), setupNote)
	return tabPage(header, u.checks, u.reimsSection(), u.reimsChecks, surface(notice))
}

// reimsSection is the Host tab's Reims vGPU header: the build in use and how to get one.
func (u *ui) reimsSection() fyne.CanvasObject {
	u.reimsChecks = container.NewVBox()
	u.reimsSummary = widget.NewLabel("")
	u.reimsSummary.TextStyle = fyne.TextStyle{Bold: true}
	u.reimsInfo = wrapped("")
	u.btnReimsInstall = widget.NewButtonWithIcon("Install Reims vGPU", theme.DownloadIcon(), func() {
		u.tabs.SelectIndex(5) // Log: the progress
		u.run("Install Reims vGPU", func() error { return core.InstallReims(context.Background(), u.logAsync) }, u.refreshReimsInfo)
	})
	u.btnReimsBuild = widget.NewButtonWithIcon("Build from source (Docker)", theme.ComputerIcon(), func() {
		dialog.ShowConfirm("Build Reims vGPU", "Build Reims vGPU's QEMU from source in a Docker container for this host's Ubuntu release? "+
			"It downloads Rust, QEMU and SDL3 sources and takes about 10 to 20 minutes the first time.", func(ok bool) {
			if ok {
				u.tabs.SelectIndex(5)
				u.run("Build Reims vGPU", func() error { return core.BuildReims(context.Background(), "", u.logAsync) }, u.refreshReimsInfo)
			}
		}, u.win)
	})
	about := caption("Reims vGPU is an experimental paravirtual GPU: Metal in macOS through the host's Vulkan driver, without passing a GPU through. " +
		"It runs on its own QEMU build, installed in <Workspace>/.reims from a prebuilt package (Ubuntu 24.04 and 26.04) or built in Docker. " +
		"Choose it per VM in Settings › Display. The checks below are warnings unless the selected VM uses it.")
	about.Wrapping = fyne.TextWrapWord
	u.refreshReimsInfo()
	return tabHeader("Reims vGPU (experimental)", "gpu", gpuTabIcon(),
		container.NewBorder(nil, nil, nil, container.NewHBox(u.btnReimsInstall, u.btnReimsBuild), u.reimsSummary), u.reimsInfo, about)
}

// refreshReimsInfo shows the Reims build in use; probing it runs QEMU, so it is off the UI thread.
func (u *ui) refreshReimsInfo() {
	go func() {
		text := "Not installed."
		if info, err := core.ReimsInstalled(); err == nil {
			text = "In use: " + info.QEMU
			if info.Build != "" {
				text += "\nBuild " + info.Build + ", upstream commit " + info.Commit
			}
			if info.AppleGFX != nil {
				text += "\n" + info.AppleGFX.Error()
			}
		}
		fyne.Do(func() { u.reimsInfo.SetText(text) })
	}()
}

func checkCard(c core.Check) fyne.CanvasObject {
	card := newItemCard("", c.Name, c.Detail)
	badge := map[core.Status]fyne.ThemeColorName{core.OK: theme.ColorNameSuccess, core.Warn: theme.ColorNameWarning,
		core.Fail: theme.ColorNameError}[c.Status]
	if badge == "" {
		badge = theme.ColorNameDisabled
	}
	card.mark(badge, statusIcon(c.Status), c.Status.String(), statusImportance(c.Status), c.Hint, c.Status == core.Fail)
	return card.obj
}

// fillChecks shows checks in box and returns how many passed.
func fillChecks(box *fyne.Container, checks []core.Check) int {
	box.RemoveAll()
	ok := 0
	for _, c := range checks {
		box.Add(checkCard(c))
		if c.Status == core.OK {
			ok++
		}
	}
	return ok
}

func (u *ui) refreshChecks() {
	all := u.host.ChecksFor(u.cfg)
	reims := u.host.ReimsChecksFor(u.cfg) // ChecksFor ends with these
	checks := all[:len(all)-len(reims)]
	ok := fillChecks(u.checks, checks)
	if ok == len(checks) {
		u.hostSummary.SetText(fmt.Sprintf("All %d checks passed: the host is ready", ok))
	} else {
		u.hostSummary.SetText(fmt.Sprintf("%d of %d checks passed", ok, len(checks)))
	}
	ok = fillChecks(u.reimsChecks, reims)
	switch {
	case ok == len(reims):
		u.reimsSummary.SetText("Ready for Reims vGPU VMs")
	case u.cfg.Reims():
		u.reimsSummary.SetText(fmt.Sprintf("%d of %d checks passed: %s cannot start yet", ok, len(reims), u.vmName()))
	default:
		u.reimsSummary.SetText(fmt.Sprintf("%d of %d checks passed (only needed for Reims vGPU VMs)", ok, len(reims)))
	}
}

func versionLabel(v core.Version) string {
	if core.ProfileFor(v.Short).Experimental {
		return v.Name + " (experimental)"
	}
	return v.Name
}

const (
	displayAppLabel    = "In the app (VNC on a private socket)"
	displayWindowLabel = "QEMU window"
)

const noGPU = "None (emulated display)"

func (u *ui) gpuTab() fyne.CanvasObject {
	u.gpuList = container.NewVBox()
	u.gpuStatus = wrapped("")
	u.gpuStatus.Wrapping = fyne.TextWrapWord
	u.gpuSelect = widget.NewSelect(nil, func(choice string) {
		addr := ""
		if choice != noGPU {
			addr, _, _ = strings.Cut(choice, " ")
		}
		if addr == u.cfg.GPU {
			return
		}
		c := u.cfg
		c.GPU = addr
		u.saveConfig(c)
	})
	u.btnReserve = widget.NewButtonWithIcon("Reserve for VFIO", theme.ConfirmIcon(), func() {
		g, ok := u.chosenGPU()
		if !ok {
			dialog.ShowInformation("GPU", "Choose an eligible GPU first.", u.win)
			return
		}
		dialog.ShowConfirm("Reserve GPU", "vfio-pci will claim "+g.Name+" at the next boot, so the host can no longer use it. "+
			"The host must show its screen on another GPU. A host reboot is required. Continue?", func(ok bool) {
			if ok {
				u.privileged("Reserve GPU", func() error { return u.host.ReserveGPU(g) })
			}
		}, u.win)
	})
	u.btnRelease = widget.NewButtonWithIcon("Release reservation", theme.CancelIcon(), func() {
		u.privileged("Release GPU", u.host.ReleaseGPU)
	})
	header := tabHeader("GPU passthrough", "gpu", gpuTabIcon(),
		container.NewBorder(nil, nil, widget.NewLabel("GPU for the VM"), nil, u.gpuSelect),
		container.NewBorder(nil, nil, nil, container.NewHBox(u.btnReserve, u.btnRelease), u.gpuStatus),
		caption("macOS-supported AMD GPUs and NVIDIA Kepler GPUs can be passed to the VM. Experimental: untested on real hardware."))
	about := widget.NewAccordion(widget.NewAccordionItem("About GPU passthrough", markdown(gpuNotes)))
	return tabPage(header, u.gpuList, u.rootPatchSection(), surface(about))
}

func (u *ui) chosenGPU() (core.GPU, bool) {
	for _, g := range u.gpus {
		if g.Addr == u.cfg.GPU && g.Eligible {
			return g, true
		}
	}
	return core.GPU{}, false
}

func (u *ui) privileged(name string, fn func() error) {
	go func() {
		err := fn()
		fyne.Do(func() {
			switch {
			case errors.Is(err, core.ErrCancelled):
				dialog.ShowInformation(name, "Cancelled: nothing was changed.", u.win)
			case err != nil:
				dialog.ShowError(err, u.win)
			default:
				dialog.ShowInformation(name, "Done. Reboot the host to apply.", u.win)
			}
			u.refresh()
		})
	}()
}

func (u *ui) refreshGPUs() {
	u.gpus = u.host.GPUs()
	u.gpuList.RemoveAll()
	options := []string{noGPU}
	selected := noGPU
	for _, g := range u.gpus {
		card := newItemCard("", g.Name, fmt.Sprintf("%s · %s · %s · driver %s · IOMMU group %s", g.Addr, g.ID, g.Compat, g.Driver, g.Group))
		switch {
		case g.Eligible && g.Compat == core.CompatKepler:
			card.mark(theme.ColorNameSuccess, theme.ConfirmIcon(), "Eligible", widget.SuccessImportance,
				"NVIDIA Kepler: native up to Big Sur; Monterey to Sequoia with root-patch mode below", g.Addr == u.cfg.GPU)
		case g.Eligible:
			card.mark(theme.ColorNameSuccess, theme.ConfirmIcon(), "Eligible", widget.SuccessImportance, "Can be passed to the VM", g.Addr == u.cfg.GPU)
		default:
			card.mark(theme.ColorNameDisabled, theme.CancelIcon(), "Not eligible", widget.LowImportance, g.Reason, false)
		}
		u.gpuList.Add(card.obj)
		if g.Eligible || g.Addr == u.cfg.GPU {
			opt := g.Addr + " " + g.Name
			options = append(options, opt)
			if g.Addr == u.cfg.GPU {
				selected = opt
			}
		}
	}
	if len(u.gpus) == 0 {
		u.gpuList.Add(surface(widget.NewLabel("No PCI display adapter found.")))
	}
	u.gpuSelect.SetOptions(options)
	u.gpuSelect.SetSelected(selected)

	status := "No GPU is reserved for VFIO."
	if ids := u.host.ReservedIDs(); ids != "" {
		status = "Reserved for vfio-pci: " + ids + ". If you just changed this, reboot the host."
	}
	if u.cfg.Reims() {
		status = "This VM uses Reims vGPU (Settings › Display): choose VMware SVGA there to pass a GPU through. " + status
	}
	u.gpuStatus.SetText(status)
}

func (u *ui) logTab() fyne.CanvasObject {
	u.log = widget.NewTextGrid()
	u.logScroll = container.NewScroll(u.log)
	return widget.NewCard("QEMU and app log", "Last 1000 lines", u.logScroll)
}

// refresh recomputes texts and buttons from the real state of the disk and the VM.
func (u *ui) refresh() {
	if u.steps == nil || u.gpuList == nil || u.header == nil || u.details == nil || u.status == nil {
		return
	}
	u.refreshChecks()
	u.refreshGPUs()
	state := u.vm.State()
	running := state != core.Stopped
	free := !u.busy && !running
	u.watchOutside()
	if !running && u.screenWin != nil && u.screenWin.FullScreen() {
		u.screenWin.SetFullScreen(false) // a stopped VM leaves nothing to show full screen
	}

	w, err := u.workspace()
	ready := err == nil && w.Exists()
	u.refreshInstall(w, err, ready, free)

	enable := func(b fyne.Disableable, on bool) {
		if on {
			b.Enable()
		} else {
			b.Disable()
		}
	}
	enable(u.btnStart, free && ready && w.HasDisk())
	enable(u.btnPowerdown, state == core.Running)
	enable(u.btnKill, running)
	_, eligible := u.chosenGPU()
	enable(u.btnReserve, !running && eligible)
	u.refreshRootPatch(running)
	enable(u.btnRelease, !running && u.host.ReservedIDs() != "")
	enable(u.btnReimsInstall, !u.busy)
	enable(u.btnReimsBuild, !u.busy)
	if running || u.cfg.Reims() { // Reims vGPU and passthrough exclude each other (Config.Validate)
		u.gpuSelect.Disable()
	} else {
		u.gpuSelect.Enable()
	}

	// The screen shows a VM another process runs (watchOutside): say so instead of "Powered Off".
	outside := state == core.Stopped && u.watching && u.watchID == u.cur.ID
	if outside {
		u.header.SetText("● Running (outside the app)")
		u.header.Importance = stateImportance(core.Running)
	} else {
		u.header.SetText("● " + stateText(state))
		u.header.Importance = stateImportance(state)
	}
	u.header.Refresh()
	idle := screenIdle
	switch {
	case u.cfg.Reims() && u.emb.h != nil, u.cfg.Reims() && u.emb.pending():
		idle = u.screen.hint.Text // the embedding sets it
	case u.cfg.Reims() && !running && u.emb.enabled():
		idle = screenReimsIdle
	case u.cfg.Reims():
		idle = screenReims
	}
	if u.screen.hint.Text != idle {
		u.screen.hint.SetText(idle)
	}
	enable(u.btnDetach, u.cfg.InApp() && !u.cfg.Reims())
	enable(u.btnNew, !running && !u.singleFile)
	enable(u.btnEdit, !running)
	enable(u.btnDelete, !running && !u.singleFile)
	enable(u.btnExport, !running)
	enable(u.btnImport, !running && !u.singleFile)
	enable(u.btnFull, u.cfg.InApp() && !u.cfg.Reims() && running)
	u.vmItem.Refresh()
	u.refreshMenu()
	u.refreshDetails()
	u.refreshStatus(state, outside)
}

// rootPatchSection is the GPU tab's root-patch mode: per VM, off by default, for an NVIDIA
// Kepler GPU on Monterey to Sequoia (see core/rootpatch.go).
func (u *ui) rootPatchSection() fyne.CanvasObject {
	u.rootPatchInfo = caption("")
	u.rootPatchInfo.Wrapping = fyne.TextWrapWord
	u.rootPatch = widget.NewCheck("Root-patch mode for an NVIDIA Kepler GPU (OpenCore Legacy Patcher)", func(on bool) {
		if u.rootPatchSetting {
			return
		}
		undo := func() { u.rootPatchSetting = true; u.rootPatch.SetChecked(!on); u.rootPatchSetting = false }
		if on {
			dialog.ShowConfirm("Turn on root-patch mode",
				"For "+u.vmName()+", OpenCore will lower System Integrity Protection to 0x803 (unsigned kexts, a writable system volume, unauthenticated root) "+
					"and load AMFIPass, RSRHelper, RestrictEvents and KDKlessWorkaround, downloaded now from OpenCore Legacy Patcher 2.5.1 and checked against their SHA-256. "+
					"Then root-patch inside macOS with OCLP (see the steps below). Turn FileVault off first. Continue?",
				func(yes bool) {
					if !yes {
						undo()
						return
					}
					target := u.cur.ID // switching VMs is blocked while this runs; checked again anyway
					u.run("Download OCLP's kexts for root-patch mode", func() error { return core.DownloadRootPatchKexts(context.Background()) }, func() {
						if u.cur.ID != target {
							return
						}
						c := u.cfg
						c.RootPatch = true
						if !u.saveConfig(c) {
							undo()
						}
					})
				}, u.win)
			return
		}
		dialog.ShowConfirm("Turn off root-patch mode",
			"Did you revert the root patch in OCLP (Post-Install Root Patch › Revert Root Patches) and restart? "+
				"Remaining root patches can prevent macOS from booting once its protections are restored at the next start.",
			func(yes bool) {
				if !yes {
					undo()
					return
				}
				c := u.cfg
				c.RootPatch = false
				if !u.saveConfig(c) {
					undo()
				}
			}, u.win)
	})
	steps := widget.NewAccordion(widget.NewAccordionItem("Root-patch steps inside macOS", markdown(rootPatchSteps)))
	return surface(container.NewVBox(widget.NewLabelWithStyle("Root-patch mode (experimental)", fyne.TextAlignLeading, fyne.TextStyle{Bold: true}),
		u.rootPatch, u.rootPatchInfo, steps))
}

func (u *ui) refreshRootPatch(running bool) {
	if u.rootPatch == nil {
		return
	}
	u.rootPatchSetting = true
	u.rootPatch.SetChecked(u.cfg.RootPatch)
	u.rootPatchSetting = false
	switch {
	case !core.RootPatchVersion(u.cfg.Version) && !u.cfg.RootPatch:
		u.rootPatchInfo.SetText("Not needed or not available for this macOS version: Big Sur and older drive Kepler GPUs natively, and Tahoe has no stable OCLP support yet.")
		u.rootPatch.Disable()
		return
	case u.cfg.RootPatch:
		u.rootPatchInfo.SetText("On for this VM: SIP 0x803, AMFIPass, RSRHelper, RestrictEvents (revblock=media), KDKlessWorkaround, ipc_control_port_options=0 -nokcmismatchpanic. " +
			"Turning it off restores full SIP at the next start.")
	default:
		u.rootPatchInfo.SetText("Off. For Monterey to Sequoia with a passed-through Kepler GPU; other VMs do not need it.")
	}
	if running || u.busy {
		u.rootPatch.Disable()
	} else {
		u.rootPatch.Enable()
	}
}

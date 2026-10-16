// Package gui is the desktop interface: a VirtualBox-style window over internal/core.
package gui

import (
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
)

var logo = fyne.NewStaticResource("logo.svg", assets.Logo)

// version is shown in the window title; Run sets it.
var version = "dev"

const homeLab = "For home lab use. macOS is Apple's software: its license only allows it on Apple-branded hardware, so make sure your use complies with it."

const manualSteps = `1. In the OpenCore menu, choose **macOS Base System**.
2. Open **Disk Utility**, enable *View › Show All Devices*, select the **QEMU HARDDISK** disk and use **Erase** (APFS, GUID).
3. Close Disk Utility and choose **Reinstall macOS**. The VM restarts a few times: in OpenCore pick **macOS Installer**, then the installed disk.
4. Once macOS boots from the disk, remove the install media here to free space.`

const videoTips = `The VM has no graphics acceleration: macOS has no driver for the emulated display or for NVIDIA GPUs, so the CPU draws the screen. To make it smoother:

1. Use **Recommended for this host** in Settings › System (more vCPUs and RAM).
2. In macOS, under *System Settings › Accessibility › Display*, turn on **Reduce motion** and **Reduce transparency**.
3. Under *Desktop & Dock*, minimize with **Scale** and turn off **Animate opening applications**.
4. Use a static wallpaper (the default Sonoma wallpaper is animated and stays blank without Metal) and keep 1920x1080.

Real acceleration needs a macOS-supported AMD GPU passed through with VFIO: see the GPU tab.`

const gpuNotes = `Gives the VM a real AMD GPU, the only way to get Metal. The GPU's display output then goes to a monitor plugged into that card; the QEMU window stays for keyboard and mouse.

- Supported: RX 460–590, Vega 56/64, Radeon VII, RX 5500/5600/5700, RX 6600/6600 XT, RX 6800/6800 XT/6900 XT.
- Not supported: RX 7000, RX 6400/6500, NVIDIA, Intel and AMD Ryzen iGPUs.
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

	checks                     *fyne.Container
	steps                      []*itemCard // the Install tab's steps
	hostSummary                *widget.Label
	installTitle               *widget.Label
	installBar                 *progressLine
	installFiles               *widget.Label
	btnStartInstall            *widget.Button
	stage                      *widget.Label
	bar                        *widget.ProgressBar
	barInf                     *widget.ProgressBarInfinite
	diskSize                   *widget.Entry
	log                        *widget.TextGrid
	logScroll                  *container.Scroll
	logLines                   []string
	gpuList                    *fyne.Container
	gpuSelect                  *widget.Select
	gpuStatus                  *widget.Label
	gpus                       []core.GPU
	header                     *widget.Label // VM state chip in the toolbar
	status                     *widget.Label // status bar
	tabs                       *container.AppTabs
	vmItem                     *widget.List
	details                    map[string]*canvas.Text
	screen                     *screen
	screenHolder               *fyne.Container
	screenWin                  fyne.Window
	btnDetach, btnFull         *toolButton
	btnNew, btnEdit, btnDelete *toolButton
	vms                        []core.VMEntry
	cur                        core.VMEntry // the selected VM
	runningID                  string       // the VM QEMU is running, if any
	singleFile                 bool         // --config: one VM file, no list management
	preview                    *canvas.Image
	previewImg                 *image.RGBA
	previewBg                  *canvas.Rectangle
	previewName                *canvas.Text
	previewIdle                *fyne.Container
	btnExport, btnImport       *toolButton
	menuFor                    map[*fyne.MenuItem]*toolButton // menu items that mirror a toolbar button
	lastPreview                time.Time

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
	lang.AddTranslationsForLocale([]byte("{}"), "en") // re-reads the language set above
	a := app.NewWithID("io.github.jniltinho.macos-kvm-gui")
	a.Settings().SetTheme(appTheme{Theme: theme.DefaultTheme(), mode: themeMode()})
	a.SetIcon(logo)
	u := &ui{win: a.NewWindow("macOS KVM " + version), host: core.SystemHost(), vm: &core.VM{}}
	err := u.loadVMs(configPath)

	u.vm.OnLog = func(s string) { fyne.Do(func() { u.appendLog(s) }) }
	u.vm.OnState = func(s core.VMState, msg string) {
		fyne.Do(func() {
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
		container.NewTabItemWithIcon("GPU", theme.ColorPaletteIcon(), u.gpuTab()),
		container.NewTabItemWithIcon("Log", theme.ListIcon(), u.logTab()),
	)
	split := container.NewHSplit(u.vmList(), u.tabs)
	split.Offset = listWidth()
	u.win.SetMainMenu(u.mainMenu())
	u.win.SetContent(container.NewBorder(top, u.statusBar(), nil, nil, split))
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
	recheck := widget.NewButtonWithIcon("Check again", theme.ViewRefreshIcon(), u.refreshChecks)
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
	return tabPage(header, u.checks, surface(notice))
}

func (u *ui) refreshChecks() {
	u.checks.RemoveAll()
	checks := u.host.Checks(u.cfg.Version)
	ok := 0
	for _, c := range checks {
		card := newItemCard("", c.Name, c.Detail)
		badge := map[core.Status]fyne.ThemeColorName{core.OK: theme.ColorNameSuccess, core.Warn: theme.ColorNameWarning,
			core.Fail: theme.ColorNameError}[c.Status]
		if badge == "" {
			badge = theme.ColorNameDisabled
		}
		card.mark(badge, statusIcon(c.Status), c.Status.String(), statusImportance(c.Status), c.Hint, c.Status == core.Fail)
		u.checks.Add(card.obj)
		if c.Status == core.OK {
			ok++
		}
	}
	if ok == len(checks) {
		u.hostSummary.SetText(fmt.Sprintf("All %d checks passed: the host is ready", ok))
	} else {
		u.hostSummary.SetText(fmt.Sprintf("%d of %d checks passed", ok, len(checks)))
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
	header := tabHeader("GPU passthrough", "gpu", theme.ColorPaletteIcon(),
		container.NewBorder(nil, nil, widget.NewLabel("GPU for the VM"), nil, u.gpuSelect),
		container.NewBorder(nil, nil, nil, container.NewHBox(u.btnReserve, u.btnRelease), u.gpuStatus),
		caption("Only macOS-supported AMD GPUs can be passed to the VM. Experimental: untested on real hardware."))
	about := widget.NewAccordion(widget.NewAccordionItem("About GPU passthrough", markdown(gpuNotes)))
	return tabPage(header, u.gpuList, surface(about))
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
		if g.Eligible {
			card.mark(theme.ColorNameSuccess, theme.ConfirmIcon(), "Eligible", widget.SuccessImportance, "Can be passed to the VM", g.Addr == u.cfg.GPU)
		} else {
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
	enable(u.btnRelease, !running && u.host.ReservedIDs() != "")
	if running {
		u.gpuSelect.Disable()
	} else {
		u.gpuSelect.Enable()
	}

	u.header.SetText("● " + stateText(state))
	u.header.Importance = stateImportance(state)
	u.header.Refresh()
	enable(u.btnDetach, u.cfg.InApp())
	enable(u.btnNew, !running && !u.singleFile)
	enable(u.btnEdit, !running)
	enable(u.btnDelete, !running && !u.singleFile)
	enable(u.btnExport, !running)
	enable(u.btnImport, !running && !u.singleFile)
	enable(u.btnFull, u.cfg.InApp() && running)
	u.vmItem.Refresh()
	u.refreshMenu()
	u.refreshDetails()
	u.refreshStatus(state)
}

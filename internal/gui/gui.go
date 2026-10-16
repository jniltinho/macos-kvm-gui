// Package gui is the desktop interface: a VirtualBox-style window over internal/core.
package gui

import (
	"context"
	"errors"
	"fmt"
	"image"
	osuser "os/user"
	"runtime"
	"strconv"
	"strings"
	"time"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/app"
	"fyne.io/fyne/v2/canvas"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/dialog"
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

1. Use **Recommended for this host** in the Settings tab (more vCPUs and RAM).
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
	installInfo                *widget.Label
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
	details                    map[string]*widget.Label
	screen                     *screen
	screenHolder               *fyne.Container
	screenWin                  fyne.Window
	btnDetach                  *widget.Button
	btnFull                    *widget.Button
	btnNew, btnEdit, btnDelete *widget.Button
	settings                   *settingsForm
	vms                        []core.VMEntry
	cur                        core.VMEntry // the selected VM
	runningID                  string       // the VM QEMU is running, if any
	singleFile                 bool         // --config: one VM file, no list management
	detailIcon                 *canvas.Image
	detailName                 *widget.Label
	detailState                *widget.Label
	detailSummary              *widget.Label
	preview                    *canvas.Image
	previewImg                 *image.RGBA
	previewBg                  *canvas.Rectangle
	previewIcon                *canvas.Image
	previewIdle                *fyne.Container
	btnExport, btnImport       *widget.Button
	lastPreview                time.Time

	btnPrepare, btnDownload, btnDisk, btnRemove *widget.Button
	btnStart, btnPowerdown, btnKill             *widget.Button
	btnReserve, btnRelease                      *widget.Button
}

// Run opens the main window and blocks until it closes.
func Run(appVersion, configPath string) {
	version = appVersion
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
		container.NewTabItemWithIcon("Settings", theme.SettingsIcon(), u.configTab()),
		container.NewTabItemWithIcon("Host", theme.HomeIcon(), u.hostTab()),
		container.NewTabItemWithIcon("GPU", theme.ColorPaletteIcon(), u.gpuTab()),
		container.NewTabItemWithIcon("Log", theme.ListIcon(), u.logTab()),
	)
	split := container.NewHSplit(u.vmList(), u.tabs)
	split.Offset = 0.14
	u.win.SetContent(container.NewBorder(top, u.statusBar(), nil, nil, split))
	u.win.Resize(fyne.NewSize(1280, 820))
	u.win.SetCloseIntercept(func() {
		if u.vm.State() != core.Stopped {
			dialog.ShowInformation("VM running", "Shut down the VM before closing the app.", u.win)
			return
		}
		u.win.Close()
	})
	u.refresh()
	if err != nil {
		dialog.ShowError(err, u.win)
	}
	u.win.ShowAndRun()
}

// section is a titled card; every tab is built from these.
func section(title, subtitle string, content ...fyne.CanvasObject) fyne.CanvasObject {
	return widget.NewCard(title, subtitle, container.NewVBox(content...))
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
	setupNote := wrapped("Set up host asks for your password once, then installs missing QEMU packages, loads the KVM module, " +
		"adds you to the kvm group and enables ignore_msrs. The libvirt and input groups are not needed.")
	setupNote.TextStyle = fyne.TextStyle{Italic: true}
	notice := wrapped(homeLab)
	notice.Importance = widget.WarningImportance
	return container.NewVScroll(container.NewVBox(
		section("Host checks", "Everything should be green before starting the VM",
			u.checks, container.NewHBox(fix, recheck), setupNote),
		section("Home lab", "", notice),
	))
}

func (u *ui) refreshChecks() {
	u.checks.RemoveAll()
	for _, c := range u.host.Checks(u.cfg.Version) {
		name := widget.NewLabel(c.Name)
		name.TextStyle = fyne.TextStyle{Bold: true}
		status := widget.NewLabel(c.Status.String())
		status.Importance = statusImportance(c.Status)
		detail := wrapped(c.Detail)
		if c.Hint != "" {
			detail.SetText(c.Detail + " — " + c.Hint)
		}
		detail.TextStyle = fyne.TextStyle{Italic: true}
		row := container.NewBorder(nil, nil, container.NewHBox(widget.NewIcon(statusIcon(c.Status)), name), status, detail)
		u.checks.Add(row)
	}
}

func versionLabel(v core.Version) string {
	if core.ProfileFor(v.Short).Experimental {
		return v.Name + " (experimental)"
	}
	return v.Name
}

// settingsForm holds the Settings tab's fields so they can be reloaded for another VM.
type settingsForm struct {
	name, ws, ram, cores, vcpus, port *widget.Entry
	version, display                  *widget.Select
	short                             map[string]string
}

const (
	displayAppLabel    = "In the app (VNC on a private socket)"
	displayWindowLabel = "QEMU window"
)

func (u *ui) configTab() fyne.CanvasObject {
	f := &settingsForm{short: map[string]string{}}
	u.settings = f
	f.name, f.ws, f.ram, f.cores, f.vcpus, f.port = widget.NewEntry(), widget.NewEntry(), widget.NewEntry(), widget.NewEntry(), widget.NewEntry(), widget.NewEntry()
	var labels []string
	for _, v := range core.Versions {
		l := versionLabel(v)
		labels = append(labels, l)
		f.short[l] = v.Short
	}
	f.version = widget.NewSelect(labels, nil)
	f.display = widget.NewSelect([]string{displayAppLabel, displayWindowLabel}, nil)
	u.loadSettings()

	form := widget.NewForm(
		widget.NewFormItem("Name", f.name),
		widget.NewFormItem("Folder", f.ws),
		widget.NewFormItem("macOS version", f.version),
		widget.NewFormItem("RAM (MiB)", f.ram),
		widget.NewFormItem("Cores", f.cores),
		widget.NewFormItem("vCPUs (total)", f.vcpus),
		widget.NewFormItem("SSH port (127.0.0.1)", f.port),
		widget.NewFormItem("Display", f.display),
	)
	form.SubmitText = "Save"
	form.OnSubmit = func() {
		if u.vm.State() != core.Stopped {
			dialog.ShowInformation("Settings", "Shut down the VM before changing the settings.", u.win)
			return
		}
		c := u.cfg // keeps fields edited elsewhere, such as the GPU
		c.Name, c.Workspace = strings.TrimSpace(f.name.Text), strings.TrimSpace(f.ws.Text)
		c.Version, c.Display = f.short[f.version.Selected], map[string]string{displayAppLabel: core.DisplayApp, displayWindowLabel: core.DisplayWindow}[f.display.Selected]
		var bad string
		for _, n := range []struct {
			name string
			e    *widget.Entry
			dst  *int
		}{{"RAM", f.ram, &c.RAMMiB}, {"Cores", f.cores, &c.Cores}, {"vCPUs", f.vcpus, &c.VCPUs}, {"SSH port", f.port, &c.SSHPort}} {
			v, err := strconv.Atoi(strings.TrimSpace(n.e.Text))
			if err != nil && bad == "" {
				bad = n.name + ": enter a whole number"
			}
			*n.dst = v
		}
		if bad == "" && c.Name == "" {
			bad = "Name: give the VM a name"
		}
		if bad != "" {
			dialog.ShowError(errors.New(bad), u.win)
			return
		}
		if u.saveConfig(c) {
			u.reloadVMs(u.cur.ID)
			dialog.ShowInformation("Settings", "Settings saved.", u.win)
		}
	}
	move := widget.NewButtonWithIcon("Move to Workspace", theme.FolderOpenIcon(), u.moveVMDialog)
	recommend := widget.NewButtonWithIcon("Recommended for this host", theme.ComputerIcon(), func() {
		r := core.Recommended(u.cfg, runtime.NumCPU(), u.host.MemTotalMiB())
		f.ram.SetText(strconv.Itoa(r.RAMMiB))
		f.cores.SetText(strconv.Itoa(r.Cores))
		f.vcpus.SetText(strconv.Itoa(r.VCPUs))
	})
	note := wrapped("Without a supported GPU, macOS draws the screen on the CPU: more vCPUs make video smoother. " +
		"Recommended only fills in the fields; click Save to apply. " +
		"The CPU model and network card follow the macOS version. Only Sonoma has been validated end to end.")
	note.TextStyle = fyne.TextStyle{Italic: true}
	modes := map[string]string{"Dark": "dark", "Light": "light", "System": "system"}
	appearance := widget.NewSelect([]string{"Dark", "Light", "System"}, func(label string) {
		m := modes[label]
		core.SetPreference("theme", m)
		fyne.CurrentApp().Settings().SetTheme(appTheme{Theme: theme.DefaultTheme(), mode: m})
	})
	for label, m := range modes {
		if m == themeMode() {
			appearance.SetSelected(label)
		}
	}
	themeNote := wrapped("Applies right away to every window. System follows the desktop's light or dark setting. MKG_THEME in the environment or .env overrides it.")
	themeNote.TextStyle = fyne.TextStyle{Italic: true}
	root := widget.NewEntry()
	root.SetText(core.WorkspaceRoot())
	rootForm := widget.NewForm(widget.NewFormItem("Workspace", root))
	rootForm.SubmitText = "Save"
	rootForm.OnSubmit = func() {
		if err := core.SetWorkspaceRoot(root.Text); err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		dialog.ShowInformation("Workspace", "Saved. New and imported VMs get a folder inside "+core.WorkspaceRoot()+".", u.win)
	}
	rootNote := wrapped("New and imported VMs get their own folder inside the Workspace (<Workspace>/<name>). VMs that already exist stay in their folder.")
	rootNote.TextStyle = fyne.TextStyle{Italic: true}
	return container.NewVScroll(container.NewVBox(
		section("Virtual machine", "Each VM has its own settings and folder", form, container.NewHBox(move)),
		section("Resources", "", container.NewHBox(recommend), note),
		section("Workspace", "The folder that holds one folder per VM", rootForm, rootNote),
		section("Appearance", "", widget.NewForm(widget.NewFormItem("Theme", appearance)), themeNote),
	))
}

// loadSettings fills the Settings tab from the selected VM.
func (u *ui) loadSettings() {
	f := u.settings
	if f == nil {
		return
	}
	f.name.SetText(u.cfg.Name)
	f.ws.SetText(u.cfg.Workspace)
	for _, v := range core.Versions {
		if v.Short == u.cfg.Version {
			f.version.SetSelected(versionLabel(v))
		}
	}
	f.ram.SetText(strconv.Itoa(u.cfg.RAMMiB))
	f.cores.SetText(strconv.Itoa(u.cfg.Cores))
	f.vcpus.SetText(strconv.Itoa(u.cfg.VCPUs))
	f.port.SetText(strconv.Itoa(u.cfg.SSHPort))
	f.display.SetSelected(map[bool]string{true: displayAppLabel, false: displayWindowLabel}[u.cfg.InApp()])
}

func (u *ui) installTab() fyne.CanvasObject {
	u.installInfo = widget.NewLabel("")
	u.installInfo.Wrapping = fyne.TextWrapWord
	u.stage = widget.NewLabel("")
	u.bar = widget.NewProgressBar()
	u.bar.Hide()
	u.barInf = widget.NewProgressBarInfinite()
	u.barInf.Stop()
	u.barInf.Hide()

	u.btnPrepare = widget.NewButtonWithIcon("Create VM folder", theme.FolderNewIcon(), func() {
		w, err := u.workspace()
		if err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		c := u.cfg
		u.run("Create VM folder", func() error { return w.Prepare(c) }, nil)
	})
	u.btnDownload = widget.NewButtonWithIcon("Download macOS recovery", theme.DownloadIcon(), func() {
		w, err := u.workspace()
		if err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		version := u.cfg.Version
		u.run("Download recovery ("+version+")", func() error {
			return w.DownloadMedia(context.Background(), version, func(p core.Progress) {
				fyne.Do(func() { u.showProgress(p) })
			}, u.logAsync)
		}, nil)
	})
	u.diskSize = widget.NewEntry()
	u.diskSize.SetText("80")
	u.btnDisk = widget.NewButtonWithIcon("Create disk", theme.StorageIcon(), func() {
		w, err := u.workspace()
		if err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		size, err := strconv.Atoi(strings.TrimSpace(u.diskSize.Text))
		if err != nil {
			dialog.ShowError(errors.New("disk size: enter a whole number of GiB"), u.win)
			return
		}
		var warning string
		u.run("Create disk", func() (err error) {
			warning, err = w.CreateDisk(context.Background(), size)
			return err
		}, func() {
			if warning != "" {
				dialog.ShowInformation("Disk created with a warning", warning, u.win)
			}
		})
	})
	u.btnRemove = widget.NewButtonWithIcon("Remove install media", theme.DeleteIcon(), func() {
		dialog.ShowConfirm("Remove install media",
			"Confirm that macOS already boots from the installed disk. BaseSystem.img will be deleted.", func(ok bool) {
				if !ok {
					return
				}
				w, err := u.workspace()
				if err != nil {
					dialog.ShowError(err, u.win)
					return
				}
				var freed int64
				u.run("Remove install media", func() (err error) {
					freed, err = w.RemoveMedia()
					return err
				}, func() {
					dialog.ShowInformation("Install media removed", fmt.Sprintf("%.2f GiB freed.", float64(freed)/(1<<30)), u.win)
				})
			}, u.win)
	})

	u.btnDownload.Importance = widget.HighImportance
	disk := container.NewBorder(nil, nil, widget.NewLabel("Size (GiB, min. 40):"), u.btnDisk, u.diskSize)
	return container.NewVScroll(container.NewVBox(
		section("Status", "", u.installInfo),
		section("1 · VM folder", "Creates the VM's folder and extracts the built-in OpenCore and OVMF", container.NewHBox(u.btnPrepare)),
		section("2 · macOS recovery", "Downloaded from Apple, signature and hashes verified", container.NewHBox(u.btnDownload), u.stage, u.bar, u.barInf),
		section("3 · Disk", "Thin qcow2: it only takes the space macOS uses", disk),
		section("4 · Install inside the VM", "", widget.NewRichTextFromMarkdown(manualSteps)),
		section("5 · Clean up", "After macOS boots from the disk", container.NewHBox(u.btnRemove)),
	))
}

func (u *ui) showInfinite(stage string) {
	u.stage.SetText(stage)
	u.bar.Hide()
	u.barInf.Show()
	u.barInf.Start()
}

func (u *ui) showProgress(p core.Progress) {
	if p.Pct < 0 {
		u.showInfinite(p.Stage)
		return
	}
	u.stage.SetText(fmt.Sprintf("%s: %.1f%%", p.Stage, p.Pct))
	u.barInf.Stop()
	u.barInf.Hide()
	u.bar.Show()
	u.bar.SetValue(p.Pct / 100)
}

const noGPU = "None (emulated display)"

func (u *ui) gpuTab() fyne.CanvasObject {
	u.gpuList = container.NewVBox()
	u.gpuStatus = widget.NewLabel("")
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
	return container.NewVScroll(container.NewVBox(
		section("Display adapters", "Only macOS-supported AMD GPUs can be passed to the VM", u.gpuList),
		section("Passthrough", "", container.NewBorder(nil, nil, widget.NewLabel("GPU for the VM:"), nil, u.gpuSelect),
			container.NewHBox(u.btnReserve, u.btnRelease), u.gpuStatus),
		section("About GPU passthrough", "", widget.NewRichTextFromMarkdown(gpuNotes)),
	))
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
		icon, verdict := theme.ErrorIcon(), widget.NewLabel("Not eligible")
		verdict.Importance = widget.DangerImportance
		reason := g.Reason
		if g.Eligible {
			icon, reason = theme.ConfirmIcon(), "Can be passed to the VM"
			verdict.SetText("Eligible")
			verdict.Importance = widget.SuccessImportance
		}
		name := wrapped(g.Name)
		name.TextStyle = fyne.TextStyle{Bold: true}
		meta := wrapped(fmt.Sprintf("%s · %s · %s · driver %s · IOMMU group %s", g.Addr, g.ID, g.Compat, g.Driver, g.Group))
		meta.TextStyle = fyne.TextStyle{Italic: true}
		why := wrapped(reason)
		u.gpuList.Add(container.NewBorder(nil, nil, widget.NewIcon(icon), verdict, container.NewVBox(name, meta, why)))
		if g.Eligible || g.Addr == u.cfg.GPU {
			opt := g.Addr + " " + g.Name
			options = append(options, opt)
			if g.Addr == u.cfg.GPU {
				selected = opt
			}
		}
	}
	if len(u.gpus) == 0 {
		u.gpuList.Add(widget.NewLabel("No PCI display adapter found."))
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
	if u.installInfo == nil || u.gpuList == nil || u.header == nil || u.details == nil || u.status == nil {
		return
	}
	u.refreshChecks()
	u.refreshGPUs()
	state := u.vm.State()
	running := state != core.Stopped
	free := !u.busy && !running

	w, err := u.workspace()
	var lines []string
	ready := err == nil && w.Exists()
	if err != nil {
		lines = append(lines, "Invalid VM folder: "+err.Error())
	} else {
		if ready {
			lines = append(lines, "VM folder "+w.Dir+": ready")
			lines = append(lines, "Boot files: "+core.AssetVersions())
		} else {
			lines = append(lines, "VM folder "+w.Dir+": does not exist yet (click Create VM folder)")
		}
		if w.HasMedia() {
			lines = append(lines, fmt.Sprintf("Install media: %s (%.2f GiB)", w.MediaVersion(), float64(core.Allocated(w.Path(core.MediaFile)))/(1<<30)))
		} else {
			lines = append(lines, "Install media: none")
		}
		if w.HasDisk() {
			lines = append(lines, fmt.Sprintf("macOS disk: %.2f GiB used on the host", float64(core.Allocated(w.Path(core.DiskFile)))/(1<<30)))
		} else {
			lines = append(lines, "macOS disk: not created yet")
		}
	}
	u.installInfo.SetText(strings.Join(lines, "\n"))

	enable := func(b *widget.Button, on bool) {
		if on {
			b.Enable()
		} else {
			b.Disable()
		}
	}
	enable(u.btnPrepare, free && err == nil && !ready)
	enable(u.btnDownload, free && err == nil)
	enable(u.btnDisk, free && ready && !w.HasDisk())
	enable(u.btnRemove, free && w.HasMedia())
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

	u.header.SetText("● " + state.String())
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
	u.refreshDetails()
	u.refreshStatus(state)
}

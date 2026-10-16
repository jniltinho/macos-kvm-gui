package gui

import (
	"errors"
	"fmt"
	"net/url"
	"runtime"
	"strconv"
	"strings"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/canvas"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/dialog"
	"fyne.io/fyne/v2/layout"
	"fyne.io/fyne/v2/theme"
	"fyne.io/fyne/v2/widget"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

// settingsDialog is VirtualBox's machine Settings window: categories on the left, the page on
// the right, OK and Cancel at the bottom. Nothing is saved until OK.
func (u *ui) settingsDialog() {
	if u.vm.State() != core.Stopped {
		dialog.ShowInformation("Settings", "Shut down the VM before changing its settings.", u.win)
		return
	}
	c := u.cfg
	p := core.ProfileFor(c.Version)

	// General.
	name := widget.NewEntry()
	name.SetText(c.Name)
	var labels []string
	short := map[string]string{}
	for _, v := range core.Versions {
		labels = append(labels, versionLabel(v))
		short[versionLabel(v)] = v.Short
	}
	osIcon := canvas.NewImageFromResource(versionIcon(c.Version))
	osIcon.FillMode = canvas.ImageFillContain
	osIcon.SetMinSize(fyne.NewSquareSize(48))
	onVersion := func() {} // set by the Display page
	version := widget.NewSelect(labels, func(l string) {
		osIcon.Resource = versionIcon(short[l])
		osIcon.Refresh()
		onVersion()
	})
	for _, v := range core.Versions {
		if v.Short == c.Version {
			version.SetSelected(versionLabel(v))
		}
	}
	var d dialog.Dialog
	move := widget.NewButtonWithIcon("Move…", theme.FolderOpenIcon(), func() { d.Hide(); u.moveVMDialog() })
	general := widget.NewForm(
		widget.NewFormItem("Name", name),
		widget.NewFormItem("Operating System", container.NewBorder(nil, nil, nil, osIcon, version)),
		widget.NewFormItem("Folder", container.NewBorder(nil, nil, nil, move, widget.NewLabel(c.Workspace))),
	)

	// System: memory and processors, like VirtualBox's sliders.
	host := u.host.MemTotalMiB()
	ramRow, ramValue, ramSet := sliderRow(core.MinRAMMiB, float64(max(host, core.MinRAMMiB*2)), 512, "MB")
	ramSet(c.RAMMiB)
	cpuRow, cpuValue, cpuSet := sliderRow(1, float64(runtime.NumCPU()), 1, "CPUs")
	cpuSet(c.VCPUs)
	threads := widget.NewSelect([]string{"1", "2"}, nil)
	threads.SetSelected(strconv.Itoa(max(c.VCPUs/max(c.Cores, 1), 1)))
	recommend := widget.NewButtonWithIcon("Recommended for this host", theme.ComputerIcon(), func() {
		r := core.Recommended(c, runtime.NumCPU(), host)
		ramSet(r.RAMMiB)
		cpuSet(r.VCPUs)
		threads.SetSelected(strconv.Itoa(r.VCPUs / max(r.Cores, 1)))
	})
	system := container.NewAppTabs(
		container.NewTabItem("Motherboard", container.NewVBox(widget.NewForm(
			widget.NewFormItem("Base Memory", ramRow),
			widget.NewFormItem("Chipset", widget.NewLabel("Q35 (ICH9)")),
			widget.NewFormItem("Firmware", widget.NewLabel("UEFI (OVMF) with OpenCore")),
		), container.NewHBox(recommend))),
		container.NewTabItem("Processor", container.NewVBox(widget.NewForm(
			widget.NewFormItem("Processors", cpuRow),
			widget.NewFormItem("Threads per core", threads),
			widget.NewFormItem("CPU Model", widget.NewLabel(strings.SplitN(p.CPU, ",", 2)[0]+" (follows the macOS version)")),
		), note("Without a supported GPU the CPU draws the screen: more processors make it smoother."))),
		container.NewTabItem("Acceleration", widget.NewForm(
			widget.NewFormItem("Hypervisor", widget.NewLabel("KVM")),
			widget.NewFormItem("Paging", widget.NewLabel("Nested paging (host CPU feature)")),
		)),
	)

	// Display.
	display := widget.NewSelect([]string{displayAppLabel, displayWindowLabel}, nil)
	display.SetSelected(map[bool]string{true: displayAppLabel, false: displayWindowLabel}[c.InApp()])
	graphics, reimsBox := graphicsSelect(display)
	graphics.SetSelected(graphicsLabel(c))
	gpuNote := note("GPU passthrough is set for this VM: choose " + noGPU + " in the GPU tab to use Reims vGPU.")
	gpuNote.Hidden = c.GPU == ""
	if c.GPU != "" { // core refuses both; the GPU tab's choice is disabled the other way round
		graphics.Disable()
	}
	// Resolution: Reims vGPU's display size; VMware SVGA keeps OpenCore's 1920x1080.
	wideNote := wrapped("Wider than 2048 px: on macOS before 26 WindowServer may crash at login (ReimsVgpu.kext loads on macOS 26 only)")
	wideNote.Importance = widget.WarningImportance
	resolution := widget.NewSelect(core.Resolutions, nil)
	resolution.SetSelected(c.ReimsResolution())
	updateResolution := func() {
		reims := graphics.Selected == graphicsReimsLabel
		if reims {
			resolution.Enable()
		} else {
			resolution.Disable()
		}
		wideNote.Hidden = !reims || !core.WideResolution(resolution.Selected) || short[version.Selected] == "tahoe"
		wideNote.Refresh()
	}
	resolution.OnChanged = func(string) { updateResolution() }
	onVersion = updateResolution
	onGraphics := graphics.OnChanged
	graphics.OnChanged = func(l string) { onGraphics(l); updateResolution() }
	updateResolution()
	displayPage := container.NewVBox(widget.NewForm(
		widget.NewFormItem("Shown", display),
		widget.NewFormItem("Graphics Controller", graphics),
		widget.NewFormItem("Resolution", resolution),
	), wideNote, gpuNote, reimsBox)

	// Storage: read-only here; the Install tab creates the disk and manages the media.
	disk, media := "not created yet", "Empty"
	if w, err := u.workspace(); err == nil {
		if w.HasDisk() {
			disk = fmt.Sprintf("mac_hdd_ng.img, %.2f GB used on the host", float64(core.Allocated(w.Path(core.DiskFile)))/(1<<30))
		}
		if w.HasMedia() {
			media = "BaseSystem.img (" + w.MediaVersion() + " recovery)"
		}
	}
	// The install media can be removed here too, once macOS boots from its disk (the same action
	// as Install step 5, with its confirmation).
	removeMedia := widget.NewButtonWithIcon("Remove…", theme.DeleteIcon(), func() {
		d.Hide()
		u.btnRemove.OnTapped()
	})
	if u.btnRemove.Disabled() {
		removeMedia.Disable()
	}
	mediaRow := container.NewBorder(nil, nil, nil, removeMedia, widget.NewLabel(media))
	storage := container.NewVBox(widget.NewForm(
		widget.NewFormItem("Controller", widget.NewLabel("SATA (ICH9 AHCI)")),
		widget.NewFormItem("SATA Port 0", widget.NewLabel(disk)),
		widget.NewFormItem("SATA Port 1", mediaRow),
		widget.NewFormItem("Boot Disk", widget.NewLabel("OpenCore (built in, read-only)")),
	), note("Remove the install media once macOS boots from its disk. Create the disk and download the media in the Install tab."))

	// Network: VirtualBox's Adapter 1–4 tabs.
	network, readNetwork := networkPage(c, func() string { return short[version.Selected] })

	pages := []struct {
		title, icon string
		fallback    fyne.Resource
		page        fyne.CanvasObject
	}{
		{"General", "general", theme.InfoIcon(), general},
		{"System", "system", theme.ComputerIcon(), system},
		{"Display", "display", theme.ViewFullScreenIcon(), displayPage},
		{"Storage", "storage", theme.StorageIcon(), storage},
		{"Audio", "audio", theme.VolumeUpIcon(), widget.NewForm(
			widget.NewFormItem("Host Driver", widget.NewLabel("QEMU default")),
			widget.NewFormItem("Controller", widget.NewLabel("Intel HD Audio")))},
		{"Network", "network", theme.MailSendIcon(), network},
		{"USB", "usb", theme.SettingsIcon(), widget.NewForm(
			widget.NewFormItem("USB Controller", widget.NewLabel("xHCI (USB 3.0)")),
			widget.NewFormItem("Devices", widget.NewLabel("USB keyboard and tablet (no host USB passthrough)")))},
	}
	body := container.NewStack()
	list := widget.NewList(func() int { return len(pages) },
		func() fyne.CanvasObject {
			return container.NewHBox(widget.NewIcon(theme.InfoIcon()), widget.NewLabel("System"))
		},
		func(i widget.ListItemID, o fyne.CanvasObject) {
			row := o.(*fyne.Container)
			row.Objects[0].(*widget.Icon).SetResource(appIcon(pages[i].icon, pages[i].fallback))
			row.Objects[1].(*widget.Label).SetText(pages[i].title)
		})
	list.OnSelected = func(i widget.ListItemID) {
		body.Objects = []fyne.CanvasObject{container.NewBorder(sectionBand(pages[i].title, appIcon(pages[i].icon, pages[i].fallback)), nil, nil, nil,
			container.NewVScroll(container.NewPadded(pages[i].page)))}
		body.Refresh()
	}
	list.Select(0)

	save := func() error {
		n := c
		n.Name = strings.TrimSpace(name.Text)
		n.Version = short[version.Selected]
		n.Display = map[string]string{displayAppLabel: core.DisplayApp, displayWindowLabel: core.DisplayWindow}[display.Selected]
		n.Graphics = graphicsValue(graphics.Selected, c.Graphics)
		if n.Reims() {
			n.Resolution = resolution.Selected
		}
		var err error
		if n.RAMMiB, err = ramValue(); err != nil {
			return fmt.Errorf("Base Memory: %w", err)
		}
		if n.VCPUs, err = cpuValue(); err != nil {
			return fmt.Errorf("Processors: %w", err)
		}
		t, _ := strconv.Atoi(threads.Selected)
		if n.VCPUs%t != 0 {
			return errors.New("Processors: with 2 threads per core, choose an even number")
		}
		n.Cores = n.VCPUs / t
		if n.Networks, n.SSHPort, err = readNetwork(); err != nil {
			return err
		}
		if n.Name == "" {
			return errors.New("Name: give the VM a name")
		}
		if err := core.SaveVM(u.cur, n); err != nil {
			return err
		}
		u.cur.Config, u.cfg = n, n
		return nil
	}
	ok := widget.NewButtonWithIcon("OK", theme.ConfirmIcon(), nil)
	ok.Importance = widget.HighImportance
	cancel := widget.NewButtonWithIcon("Cancel", theme.CancelIcon(), nil)
	sizer := canvas.NewRectangle(nil)
	sizer.SetMinSize(fyne.NewSize(200, 0))
	content := container.NewBorder(nil,
		container.NewVBox(widget.NewSeparator(), container.NewHBox(layout.NewSpacer(), ok, cancel)), nil, nil,
		container.NewBorder(nil, nil, container.NewHBox(container.NewStack(sizer, list), widget.NewSeparator()), nil, body))
	d = dialog.NewCustomWithoutButtons(u.vmName()+" - Settings", content, u.win)
	cancel.OnTapped = d.Hide
	ok.OnTapped = func() {
		if err := save(); err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		d.Hide()
		u.reloadVMs(u.cur.ID)
	}
	d.Resize(fyne.NewSize(900, 560))
	d.Show()
}

// preferencesDialog holds the app-wide settings, like VirtualBox's Preferences: the Workspace
// (the default machine folder) and the theme.
func (u *ui) preferencesDialog() {
	root := widget.NewEntry()
	root.SetText(core.WorkspaceRoot())
	modes := map[string]string{"Dark": "dark", "Light": "light", "System": "system"}
	themeSel := widget.NewSelect([]string{"Dark", "Light", "System"}, nil)
	for label, m := range modes {
		if m == themeMode() {
			themeSel.SetSelected(label)
		}
	}
	form := container.NewVBox(
		sectionBand("General", appIcon("general", theme.InfoIcon())),
		widget.NewForm(widget.NewFormItem("Workspace", root)),
		note("New and imported VMs get their own folder inside the Workspace (<Workspace>/<name>). Existing VMs stay where they are; use Machine › Move to move one."),
		sectionBand("Interface", appIcon("display", theme.ViewFullScreenIcon())),
		widget.NewForm(widget.NewFormItem("Theme", themeSel)),
		note("System follows the desktop's light or dark setting. theme in settings.yaml (or MKG_THEME) overrides it."),
		sectionBand("Settings file", appIcon("general", theme.DocumentIcon())),
		container.NewHBox(
			widget.NewButton("Open settings.yaml", func() {
				core.LoadExtraKexts() // creates the default on first use
				fyne.CurrentApp().OpenURL(&url.URL{Scheme: "file", Path: core.SettingsFile()})
			}),
			widget.NewButton("Reset to default", func() {
				dialog.ShowConfirm("Reset settings.yaml", "Overwrite "+core.SettingsFile()+" with the default, which sets nothing?", func(ok bool) {
					if ok {
						if err := core.ResetSettings(); err != nil {
							dialog.ShowError(err, u.win)
						}
					}
				}, u.win)
			}),
		),
		note(core.SettingsFile()+" holds the theme, the boot files override and the extra kexts OpenCore loads in every VM (for example a patch kext a GPU needs; the app ships none). Environment variables win over it."),
	)
	d := dialog.NewCustomConfirm("Preferences", "OK", "Cancel", form, func(ok bool) {
		if !ok {
			return
		}
		if err := core.SetWorkspaceRoot(root.Text); err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		m := modes[themeSel.Selected]
		core.SetPreference("theme", m)
		fyne.CurrentApp().Settings().SetTheme(appTheme{Theme: theme.DefaultTheme(), mode: themeMode()})
	}, u.win)
	d.Resize(fyne.NewSize(640, 0))
	d.Show()
}

func note(s string) *widget.Label {
	l := wrapped(s)
	l.TextStyle = fyne.TextStyle{Italic: true}
	return l
}

const (
	graphicsSVGALabel  = "VMware SVGA"
	graphicsReimsLabel = "Reims vGPU (experimental)"
)

func graphicsLabel(c core.Config) string {
	if c.Reims() {
		return graphicsReimsLabel
	}
	return graphicsSVGALabel
}

// graphicsValue is Config.Graphics for a Graphics choice, keeping an explicit vmware-svga.
func graphicsValue(label, old string) string {
	if label == graphicsReimsLabel {
		return core.GraphicsReims
	}
	if old == core.GraphicsReims {
		return ""
	}
	return old
}

// METAL_ERROR_MODE 5, set once per macOS install: without it the desktop never appears on Reims vGPU.
const (
	metalDesktopCmd  = "sudo defaults write /Library/Preferences/.GlobalPreferences METAL_ERROR_MODE -int 5"
	metalRecoveryCmd = `cd /Volumes/*Data/Library/Preferences
defaults write "$PWD/.GlobalPreferences" METAL_ERROR_MODE -int 5
chmod 644 .GlobalPreferences.plist
reboot`
)

// graphicsSelect is the Graphics Controller choice and the Reims vGPU guidance it shows when
// Reims is selected; display (the Shown choice) does not apply to Reims VMs and is disabled.
func graphicsSelect(display *widget.Select) (*widget.Select, fyne.CanvasObject) {
	cmd := func(text string) fyne.CanvasObject {
		e := widget.NewEntry()
		if strings.Contains(text, "\n") {
			e = widget.NewMultiLineEntry()
			e.SetMinRowsVisible(4)
		}
		e.SetText(text)
		e.TextStyle = fyne.TextStyle{Monospace: true}
		copyBtn := widget.NewButtonWithIcon("Copy", theme.ContentCopyIcon(), func() {
			fyne.CurrentApp().Clipboard().SetContent(text)
		})
		return container.NewBorder(nil, nil, nil, container.NewVBox(copyBtn), e)
	}
	missing := wrapped("Reims vGPU is not installed on this host: install it or build it in the Host tab.")
	missing.Importance = widget.WarningImportance
	box := container.NewVBox(
		note("Upstream measured macOS 26 (Tahoe) guests only; this project validated macOS Sequoia 15.8.1 (Apple Paravirtualized Graphics, Metal, 1920x1080). "+
			"Other versions are untested. Switching back to VMware SVGA keeps the VM as it was."),
		note("The screen opens in its own Reims vGPU window: the Screen tab is not used, the Preview still updates."),
		missing,
		widget.NewLabelWithStyle("Set METAL_ERROR_MODE once per macOS install, or the desktop never appears", fyne.TextAlignLeading, fyne.TextStyle{Bold: true}),
		caption("On a working desktop (before switching), in Terminal:"),
		cmd(metalDesktopCmd),
		caption("On a new install, from the recovery's Utilities › Terminal (after macOS is installed):"),
		cmd(metalRecoveryCmd),
		note("Tahoe VMs load ReimsVgpu.kext (downloaded at the first start), which macOS 26 needs for displays wider than 2048 pixels (upstream's off switch is the boot argument -reimsvgpuoff)."),
	)
	sel := widget.NewSelect([]string{graphicsSVGALabel, graphicsReimsLabel}, func(l string) {
		on := l == graphicsReimsLabel
		box.Hidden = !on
		if on {
			_, err := core.ReimsQEMU()
			missing.Hidden = err == nil
			display.Disable()
		} else {
			display.Enable()
		}
		box.Refresh()
	})
	box.Hide()
	return sel, box
}

package gui

import (
	"fmt"
	"image"
	"image/color"
	"strings"
	"time"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/canvas"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/dialog"
	"fyne.io/fyne/v2/layout"
	"fyne.io/fyne/v2/theme"
	"fyne.io/fyne/v2/widget"

	"github.com/jniltinho/macos-kvm-gui/internal/assets"
	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

// VirtualBox-style layout: a toolbar on top, the VM list on the left, tabs on the right
// (Details, Screen and the tools) and a status bar at the bottom.

func (u *ui) toolbar() fyne.CanvasObject {
	icon := canvas.NewImageFromResource(logo)
	icon.FillMode = canvas.ImageFillContain
	icon.SetMinSize(fyne.NewSquareSize(32))
	title := widget.NewLabel("macOS KVM")
	title.TextStyle = fyne.TextStyle{Bold: true}
	title.SizeName = theme.SizeNameSubHeadingText

	u.btnNew = widget.NewButtonWithIcon("New", theme.ContentAddIcon(), u.newVMDialog)
	u.btnEdit = widget.NewButtonWithIcon("Edit", theme.DocumentCreateIcon(), func() { u.tabs.SelectIndex(3) }) // Settings
	u.btnDelete = widget.NewButtonWithIcon("Delete", theme.DeleteIcon(), u.deleteVMDialog)
	u.btnExport = widget.NewButtonWithIcon("Export", theme.UploadIcon(), u.exportVMDialog)
	u.btnImport = widget.NewButtonWithIcon("Import", theme.DownloadIcon(), u.importVMDialog)
	u.btnStart = widget.NewButtonWithIcon("Start", theme.MediaPlayIcon(), u.start)
	u.btnPowerdown = widget.NewButtonWithIcon("Shut down", theme.LogoutIcon(), func() {
		go func() {
			err := u.vm.Powerdown()
			fyne.Do(func() {
				if err != nil {
					dialog.ShowError(err, u.win)
					return
				}
				u.appendLog("[VM] ACPI power button sent to macOS")
				dialog.ShowInformation("Shut down",
					"The power button was sent to macOS. Many macOS versions treat it as Sleep, which does nothing in a VM.\n\n"+
						"If macOS keeps running, shut it down from the Apple menu (Apple › Shut Down) inside the VM: "+
						"the app notices when it powers off. Use Force stop only if macOS is frozen.", u.win)
			})
		}()
	})
	u.btnKill = widget.NewButtonWithIcon("Force stop", theme.CancelIcon(), func() {
		dialog.ShowConfirm("Force stop", "This is like pulling the power cord: unsaved data in macOS will be lost. Continue?",
			func(ok bool) {
				if ok {
					if err := u.vm.Kill(); err != nil {
						dialog.ShowError(err, u.win)
					}
				}
			}, u.win)
	})
	u.btnDetach = widget.NewButtonWithIcon("Open in window", theme.ViewRestoreIcon(), func() { u.detachScreen(false) })
	u.btnFull = widget.NewButtonWithIcon("Full screen", theme.ViewFullScreenIcon(), func() { u.detachScreen(true) })
	u.btnStart.Importance = widget.HighImportance
	u.btnPowerdown.Importance = widget.WarningImportance
	u.btnKill.Importance = widget.DangerImportance

	u.header = widget.NewLabel("")
	u.header.TextStyle = fyne.TextStyle{Bold: true}
	// Everything sits on one centered row: brand, three button groups (VM, run, screen) and the
	// state chip, over a band like VirtualBox's toolbar.
	group := func(objs ...fyne.CanvasObject) fyne.CanvasObject { return container.NewHBox(objs...) }
	// The band and separators are plain rectangles, so they are recolored when the theme changes.
	var lines []*canvas.Rectangle
	gap := func() fyne.CanvasObject {
		r := canvas.NewRectangle(theme.Color(theme.ColorNameSeparator))
		r.SetMinSize(fyne.NewSize(1, 28))
		lines = append(lines, r)
		return container.NewCenter(r)
	}
	brand := container.NewHBox(container.NewCenter(icon), container.NewCenter(title))
	row := container.NewHBox(
		brand, gap(),
		group(u.btnNew, u.btnEdit, u.btnDelete, u.btnExport, u.btnImport), gap(),
		group(u.btnStart, u.btnPowerdown, u.btnKill), gap(),
		group(u.btnDetach, u.btnFull),
	)
	band := canvas.NewRectangle(theme.Color(theme.ColorNameHeaderBackground))
	fyne.CurrentApp().Settings().AddListener(func(fyne.Settings) {
		band.FillColor = theme.Color(theme.ColorNameHeaderBackground)
		band.Refresh()
		for _, r := range lines {
			r.FillColor = theme.Color(theme.ColorNameSeparator)
			r.Refresh()
		}
	})
	bar := container.NewStack(band, container.NewPadded(container.NewBorder(nil, nil, row, container.NewCenter(u.header))))
	return container.NewVBox(bar, widget.NewSeparator())
}

// vmList is the left pane. The app manages one VM per configuration, shown like VirtualBox's list.
func (u *ui) vmList() fyne.CanvasObject {
	u.vmItem = widget.NewList(
		func() int { return len(u.vms) },
		func() fyne.CanvasObject {
			img := canvas.NewImageFromResource(versionIcon(u.cfg.Version))
			img.FillMode = canvas.ImageFillContain
			img.SetMinSize(fyne.NewSquareSize(40))
			name := widget.NewLabel("")
			name.TextStyle = fyne.TextStyle{Bold: true}
			name.Truncation = fyne.TextTruncateEllipsis
			sub := widget.NewLabel("")
			sub.SizeName = theme.SizeNameCaptionText
			sub.Truncation = fyne.TextTruncateEllipsis
			state := widget.NewLabel("")
			state.SizeName = theme.SizeNameCaptionText
			// Spacers center the lines vertically while keeping them left-aligned; zero spacing
			// keeps name, details and state together like VirtualBox's list.
			text := container.New(layout.NewCustomPaddedVBoxLayout(-8), layout.NewSpacer(), name, sub, state, layout.NewSpacer())
			return container.NewBorder(nil, nil, container.NewCenter(img), nil, text)
		},
		func(id widget.ListItemID, o fyne.CanvasObject) {
			if id >= len(u.vms) {
				return
			}
			e := u.vms[id]
			row := o.(*fyne.Container)
			box := row.Objects[0].(*fyne.Container)
			img := row.Objects[1].(*fyne.Container).Objects[0].(*canvas.Image)
			name, sub, state := box.Objects[1].(*widget.Label), box.Objects[2].(*widget.Label), box.Objects[3].(*widget.Label)
			if t := fmt.Sprintf("%s · %d GiB · %d vCPUs", strings.TrimPrefix(versionTitle(e.Config.Version), "macOS "), e.Config.RAMMiB/1024, e.Config.VCPUs); sub.Text != t {
				sub.SetText(t)
			}
			if r := versionIcon(e.Config.Version); img.Resource != r {
				img.Resource = r
				img.Refresh()
			}
			if n := e.DisplayName(); name.Text != n {
				name.SetText(n)
			}
			s := core.Stopped
			if e.ID == u.runningID {
				s = u.vm.State()
			}
			t, imp := "● "+s.String(), stateImportance(s)
			if s == core.Stopped {
				if _, ok := core.FindRunning(e.Config); ok { // e.g. started by OSX-KVM's script
					t, imp = "● Running (outside the app)", widget.WarningImportance
				}
			}
			if state.Text != t || state.Importance != imp {
				state.Text, state.Importance = t, imp
				state.Refresh()
			}
		},
	)
	u.vmItem.OnSelected = u.selectVM
	u.selectRow()
	width := canvas.NewRectangle(color.Transparent) // keeps the pane wide enough for the name and state
	width.SetMinSize(fyne.NewSize(200, 0))
	return container.NewStack(width, u.vmItem)
}

// versionIcon is the version's own icon, or the app logo when there is none.
func versionIcon(short string) fyne.Resource {
	if b, err := assets.VersionIcons.ReadFile("logo/versions/" + short + ".svg"); err == nil {
		return fyne.NewStaticResource(short+".svg", b)
	}
	return logo
}

// versionTitle turns "sonoma" into "macOS Sonoma 14".
func versionTitle(short string) string {
	for _, v := range core.Versions {
		if v.Short == short {
			name, num, _ := strings.Cut(strings.TrimSuffix(v.Name, ")"), " (")
			return "macOS " + name + " " + num
		}
	}
	return "macOS " + short
}

func (u *ui) vmName() string { return u.cur.DisplayName() }

func stateImportance(s core.VMState) widget.Importance {
	switch s {
	case core.Running:
		return widget.SuccessImportance
	case core.Paused:
		return widget.WarningImportance
	}
	return widget.MediumImportance
}

// detailsTab mirrors VirtualBox's Details pane: a header with the VM's icon, name, state and a
// live preview, then one compact block per hardware area.
func (u *ui) detailsTab() fyne.CanvasObject {
	u.details = map[string]*widget.Label{}
	block := func(title string, icon fyne.Resource, rows ...string) fyne.CanvasObject {
		form := widget.NewForm()
		for _, r := range rows {
			l := widget.NewLabel("")
			l.Wrapping = fyne.TextWrapWord
			u.details[r] = l
			form.Append(r, l)
		}
		head := container.NewHBox(widget.NewIcon(icon), widget.NewLabelWithStyle(title, fyne.TextAlignLeading, fyne.TextStyle{Bold: true}))
		return widget.NewCard("", "", container.NewBorder(container.NewVBox(head, widget.NewSeparator()), nil, nil, nil, form))
	}

	u.detailIcon = canvas.NewImageFromResource(versionIcon(u.cfg.Version))
	u.detailIcon.FillMode = canvas.ImageFillContain
	u.detailIcon.SetMinSize(fyne.NewSquareSize(96))
	u.detailName = widget.NewLabel("")
	u.detailName.TextStyle = fyne.TextStyle{Bold: true}
	u.detailName.SizeName = theme.SizeNameHeadingText
	u.detailState = widget.NewLabel("")
	u.detailState.TextStyle = fyne.TextStyle{Bold: true}
	u.detailSummary = widget.NewLabel("")
	u.detailSummary.Wrapping = fyne.TextWrapWord

	// 16:9 box: the live screen fills it while running; when stopped, a small icon and a caption.
	u.preview = canvas.NewImageFromImage(image.NewRGBA(image.Rect(0, 0, 1, 1)))
	u.preview.FillMode = canvas.ImageFillContain
	u.preview.ScaleMode = canvas.ImageScaleFastest
	u.preview.SetMinSize(fyne.NewSize(320, 180))
	u.previewBg = canvas.NewRectangle(color.Black)
	u.previewIcon = canvas.NewImageFromResource(versionIcon(u.cfg.Version))
	u.previewIcon.FillMode = canvas.ImageFillContain
	u.previewIcon.SetMinSize(fyne.NewSquareSize(72))
	caption := widget.NewLabel("Not running")
	caption.Alignment = fyne.TextAlignCenter
	caption.TextStyle = fyne.TextStyle{Italic: true}
	u.previewIdle = container.NewCenter(container.NewVBox(container.NewCenter(u.previewIcon), caption))
	previewBox := container.NewStack(u.previewBg, u.preview, u.previewIdle)
	previewCard := widget.NewCard("", "", container.NewVBox(
		widget.NewLabelWithStyle("Preview", fyne.TextAlignLeading, fyne.TextStyle{Bold: true}), previewBox))

	header := widget.NewCard("", "", container.NewBorder(nil, nil, u.detailIcon, nil,
		container.NewVBox(container.NewHBox(u.detailName, u.detailState), u.detailSummary)))

	grid := container.NewGridWithColumns(3,
		block("General", theme.InfoIcon(), "Name", "Operating system", "Mac model"),
		block("System", theme.ComputerIcon(), "Base memory", "Processors", "CPU model"),
		block("Display", theme.ViewFullScreenIcon(), "Shown", "Graphics", "Resolution"),
		block("Storage", theme.StorageIcon(), "Disk", "Install media", "Boot"),
		block("Network", theme.MailSendIcon(), "Adapter", "SSH"),
		block("GPU passthrough", theme.ColorPaletteIcon(), "GPU", "Acceleration"),
	)
	tip := widget.NewLabel("Tip: without a passed-through AMD GPU the CPU draws the screen. More vCPUs (Settings › Recommended), " +
		"Reduce motion and Reduce transparency in macOS, and a static wallpaper keep it smooth.")
	tip.Wrapping = fyne.TextWrapWord
	tip.TextStyle = fyne.TextStyle{Italic: true}
	return container.NewVScroll(container.NewVBox(
		container.NewBorder(nil, nil, nil, previewCard, header),
		grid,
		tip,
	))
}

func (u *ui) refreshDetails() {
	set := func(k, v string) { u.details[k].SetText(v) }
	p := core.ProfileFor(u.cfg.Version)
	set("Name", u.vmName())
	set("Mac model", core.SMBIOSModel(u.cfg.Version)+" (SMBIOS)")
	set("Acceleration", map[bool]string{true: "Metal through the passed-through GPU", false: "None: the CPU draws the screen"}[u.cfg.GPU != ""])
	state := u.vm.State()
	u.detailName.SetText(u.vmName())
	u.detailState.SetText("● " + state.String())
	u.detailState.Importance = stateImportance(state)
	u.detailState.Refresh()
	if r := versionIcon(u.cfg.Version); u.detailIcon.Resource != r {
		u.detailIcon.Resource = r
		u.detailIcon.Refresh()
	}
	if r := versionIcon(u.cfg.Version); u.previewIcon.Resource != r {
		u.previewIcon.Resource = r
		u.previewIcon.Refresh()
	}
	if state == core.Stopped { // the live preview ends with the VM
		u.preview.Hide()
		u.previewBg.Hide()
		u.previewIdle.Show()
	}
	set("Operating system", map[bool]string{true: versionTitle(u.cfg.Version) + " (experimental)", false: versionTitle(u.cfg.Version)}[p.Experimental])
	set("Base memory", fmt.Sprintf("%d MiB", u.cfg.RAMMiB))
	set("Processors", fmt.Sprintf("%d vCPUs (%d cores × %d threads)", u.cfg.VCPUs, u.cfg.Cores, u.cfg.VCPUs/max(u.cfg.Cores, 1)))
	set("CPU model", strings.SplitN(p.CPU, ",", 2)[0])
	set("Shown", map[bool]string{true: "In the app (VNC on a private socket)", false: "QEMU window"}[u.cfg.InApp()])
	set("Graphics", "VMware SVGA (no acceleration)")
	set("Resolution", "1920x1080")
	set("Adapter", p.NIC+" (NAT)")
	set("SSH", fmt.Sprintf("127.0.0.1:%d → guest port 22", u.cfg.SSHPort))
	gpu := "None"
	if u.cfg.GPU != "" {
		gpu = u.cfg.GPU + " (vfio-pci)"
	}
	set("GPU", gpu)
	disk, media := "not created yet", "none"
	if w, err := u.workspace(); err == nil {
		if w.HasDisk() {
			disk = fmt.Sprintf("mac_hdd_ng.img, %.2f GiB used on the host", float64(core.Allocated(w.Path(core.DiskFile)))/(1<<30))
		}
		if w.HasMedia() {
			media = "BaseSystem.img (" + w.MediaVersion() + ")"
		}
	}
	exp := ""
	if core.ProfileFor(u.cfg.Version).Experimental {
		exp = " (experimental)"
	}
	u.detailSummary.SetText(fmt.Sprintf("%s%s · %s · %d MiB · %d vCPUs\nFolder %s", versionTitle(u.cfg.Version), exp,
		core.SMBIOSModel(u.cfg.Version), u.cfg.RAMMiB, u.cfg.VCPUs, u.cfg.Workspace))
	set("Disk", disk)
	set("Install media", media)
	set("Boot", "OpenCore (built in)")
}

// screenTab holds the live VM screen; "Open in window" moves it to its own window.
func (u *ui) screenTab() fyne.CanvasObject {
	u.screen = newScreen()
	u.screen.onFrame = func(f *image.RGBA) {
		if time.Since(u.lastPreview) < time.Second/5 { // the preview only needs a few frames per second
			return
		}
		u.lastPreview = time.Now()
		// The preview gets its own small image: sharing the screen's buffer between two canvas
		// images made Fyne recycle a texture that other widgets were still drawing with.
		u.previewImg = thumbnail(f, u.previewImg)
		u.preview.Image = u.previewImg
		u.previewIdle.Hide()
		u.previewBg.Show()
		u.preview.Show()
		u.preview.Refresh()
	}
	u.screenHolder = container.NewStack(u.screen)
	return u.screenHolder
}

func (u *ui) statusBar() fyne.CanvasObject {
	u.status = widget.NewLabel("")
	u.status.TextStyle = fyne.TextStyle{Italic: true}
	return container.NewVBox(widget.NewSeparator(), u.status)
}

func (u *ui) refreshStatus(s core.VMState) {
	text := "● " + s.String() + "  ·  Display: "
	if u.cfg.InApp() {
		text += "in the app. Click the screen to type; keys go to macOS while it has focus. Ctrl+Alt+F leaves full screen."
	} else {
		text += "QEMU window (change it in Settings)."
	}
	u.status.SetText(text)
}

// start launches the VM and, with the in-app display, connects the screen.
func (u *ui) start() {
	u.runningID = u.cur.ID
	if err := u.vm.Start(u.cfg); err != nil {
		dialog.ShowError(err, u.win)
		u.refresh()
		return
	}
	u.refresh()
	if !u.cfg.InApp() {
		return
	}
	u.tabs.SelectIndex(1) // Screen
	go func() {
		v, err := core.DialVNC(u.vm.VNCSocket())
		if err != nil {
			u.logAsync("screen: " + err.Error())
			return
		}
		fyne.Do(func() { u.screen.attach(v) })
		if err := v.Run(); err != nil && u.vm.State() != core.Stopped {
			u.logAsync("screen: " + err.Error())
		}
		v.Close()
		fyne.Do(u.screen.detach)
	}()
}

// thumbnail scales src down to 320 px wide (nearest neighbour) into dst, reusing it when it fits.
func thumbnail(src, dst *image.RGBA) *image.RGBA {
	sw, sh := src.Bounds().Dx(), src.Bounds().Dy()
	if sw == 0 || sh == 0 {
		return dst
	}
	w := 320
	h := max(sh*w/sw, 1)
	if dst == nil || dst.Bounds().Dx() != w || dst.Bounds().Dy() != h {
		dst = image.NewRGBA(image.Rect(0, 0, w, h))
	}
	for y := range h {
		sy := y * sh / h
		for x := range w {
			so := src.PixOffset(x*sw/w, sy)
			copy(dst.Pix[dst.PixOffset(x, y):], src.Pix[so:so+4])
		}
	}
	return dst
}

// detachScreen moves the screen to its own window, full screen or not. In that window the
// host key (Ctrl+Alt+F) toggles full screen; closing it brings the screen back to the tab.
func (u *ui) detachScreen(full bool) {
	if u.screenWin != nil {
		u.screenWin.SetFullScreen(full)
		u.screenWin.RequestFocus()
		return
	}
	hint := widget.NewLabel("The screen is open in its own window. Close that window to bring it back here.")
	hint.Alignment = fyne.TextAlignCenter
	u.screenHolder.Objects = []fyne.CanvasObject{container.NewCenter(hint)}
	u.screenHolder.Refresh()
	w := fyne.CurrentApp().NewWindow(u.vmName() + " — macOS KVM")
	w.SetContent(u.screen)
	w.Resize(fyne.NewSize(1280, 760))
	w.SetOnClosed(func() {
		u.screenWin = nil
		u.screen.onHostKey = nil
		u.screenHolder.Objects = []fyne.CanvasObject{u.screen}
		u.screenHolder.Refresh()
	})
	u.screen.onHostKey = func() {
		if w.FullScreen() {
			w.SetFullScreen(false)
		} else {
			w.Close()
		}
	}
	u.screenWin = w
	w.SetFullScreen(full)
	w.Show()
	w.Canvas().Focus(u.screen)
}

package gui

import (
	"fmt"
	"image"
	"image/color"
	"net/url"
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
	u.btnNew = newToolButton("New", appIcon("new", theme.ContentAddIcon()), theme.ColorNamePrimary, u.newVMDialog)
	u.btnImport = newToolButton("Import", appIcon("import", theme.DownloadIcon()), theme.ColorNameSuccess, u.importVMDialog)
	u.btnEdit = newToolButton("Settings", appIcon("settings", theme.SettingsIcon()), theme.ColorNameWarning, u.settingsDialog)
	u.btnDelete = newToolButton("Remove", appIcon("remove", theme.DeleteIcon()), theme.ColorNameError, u.deleteVMDialog)
	u.btnExport = newToolButton("Export", appIcon("export", theme.UploadIcon()), theme.ColorNamePrimary, u.exportVMDialog)
	u.btnStart = newToolButton("Start", appIcon("start", theme.MediaPlayIcon()), theme.ColorNameSuccess, u.start)
	u.btnPowerdown = newToolButton("Shut down", appIcon("shutdown", powerIcon), theme.ColorNameWarning, u.powerdown)
	u.btnKill = newToolButton("Force stop", appIcon("forcestop", theme.CancelIcon()), theme.ColorNameError, u.forceStop)
	u.btnDetach = newToolButton("Window", appIcon("window", theme.ViewRestoreIcon()), theme.ColorNamePrimary, func() { u.detachScreen(false) })
	u.btnFull = newToolButton("Full screen", appIcon("fullscreen", theme.ViewFullScreenIcon()), theme.ColorNamePrimary, func() { u.detachScreen(true) })

	gap := func() fyne.CanvasObject {
		r := themedRect(theme.ColorNameSeparator)
		r.SetMinSize(fyne.NewSize(1, 40))
		return container.NewCenter(r)
	}
	// VirtualBox's order: New, Open | Settings, Discard | Start. Export lives in the File menu.
	row := container.NewHBox(
		u.btnNew, u.btnImport, gap(),
		u.btnEdit, u.btnDelete, gap(),
		u.btnStart, u.btnPowerdown, u.btnKill, gap(),
		u.btnDetach, u.btnFull,
	)
	u.header = widget.NewLabel("") // the VM state, at the right like VirtualBox's tools
	u.header.TextStyle = fyne.TextStyle{Bold: true}
	return container.NewVBox(container.NewBorder(nil, nil, row, container.NewCenter(u.header)), widget.NewSeparator())
}

// powerdown sends the ACPI power button and explains macOS's usual Sleep reaction.
func (u *ui) powerdown() {
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
}

func (u *ui) forceStop() {
	dialog.ShowConfirm("Force stop", "This is like pulling the power cord: unsaved data in macOS will be lost. Continue?",
		func(ok bool) {
			if ok {
				if err := u.vm.Kill(); err != nil {
					dialog.ShowError(err, u.win)
				}
			}
		}, u.win)
}

// mainMenu is VirtualBox's File / Machine / Help menu bar. Items act through the toolbar
// buttons, so they follow the same enabled state.
func (u *ui) mainMenu() *fyne.MainMenu {
	u.menuFor = map[*fyne.MenuItem]*toolButton{}
	item := func(label string, b *toolButton) *fyne.MenuItem {
		m := fyne.NewMenuItem(label, b.trigger)
		u.menuFor[m] = b
		return m
	}
	link := func(label, to string) *fyne.MenuItem {
		return fyne.NewMenuItem(label, func() {
			if p, err := url.Parse(to); err == nil {
				fyne.CurrentApp().OpenURL(p)
			}
		})
	}
	docs := link("Online Documentation", "https://github.com/jniltinho/macos-kvm-gui/tree/main/docs")
	site := link("Web Site", "https://github.com/jniltinho/macos-kvm-gui")
	issues := link("Bug Tracker", "https://github.com/jniltinho/macos-kvm-gui/issues")
	about := fyne.NewMenuItem("About macOS KVM", func() {
		dialog.ShowInformation("About macOS KVM", "macOS KVM "+version+"\n\nBoot files: "+core.AssetVersions()+"\n\n"+homeLab, u.win)
	})
	return fyne.NewMainMenu(
		fyne.NewMenu("File",
			fyne.NewMenuItem("Preferences…", u.preferencesDialog),
			fyne.NewMenuItemSeparator(),
			item("Import VM…", u.btnImport), item("Export VM…", u.btnExport)),
		fyne.NewMenu("Machine",
			item("New…", u.btnNew), item("Settings…", u.btnEdit),
			u.moveItem(),
			item("Remove…", u.btnDelete),
			fyne.NewMenuItemSeparator(),
			item("Start", u.btnStart), item("Shut down", u.btnPowerdown), item("Force stop", u.btnKill),
			fyne.NewMenuItemSeparator(),
			item("Show in a window", u.btnDetach), item("Full screen", u.btnFull)),
		fyne.NewMenu("Help", docs, site, issues, fyne.NewMenuItemSeparator(), about),
	)
}

// moveItem follows the Settings button's state, since moving needs a stopped VM too.
func (u *ui) moveItem() *fyne.MenuItem {
	m := fyne.NewMenuItem("Move…", func() {
		if !u.btnEdit.Disabled() {
			u.moveVMDialog()
		}
	})
	u.menuFor[m] = u.btnEdit
	return m
}

// refreshMenu greys out the menu items whose toolbar button is disabled, like VirtualBox.
func (u *ui) refreshMenu() {
	changed := false
	for m, b := range u.menuFor {
		if m.Disabled != b.Disabled() {
			m.Disabled, changed = b.Disabled(), true
		}
	}
	if changed && u.win.MainMenu() != nil {
		u.win.MainMenu().Refresh()
	}
}

// appIcon is one of the app's own icons (internal/assets/icons), or fallback when it is missing.
func appIcon(name string, fallback fyne.Resource) fyne.Resource {
	if b, err := assets.Icons.ReadFile("icons/" + name + ".svg"); err == nil {
		return fyne.NewStaticResource(name+".svg", b)
	}
	return fallback
}

// stateIcon is the small symbol before a VM's state, like VirtualBox's list.
func stateIcon(s core.VMState) fyne.Resource {
	switch s {
	case core.Running:
		return appIcon("state-running", theme.NewColoredResource(theme.MediaPlayIcon(), theme.ColorNameSuccess))
	case core.Paused:
		return appIcon("state-paused", theme.NewColoredResource(theme.MediaPauseIcon(), theme.ColorNameWarning))
	}
	return appIcon("state-off", theme.NewColoredResource(powerIcon, theme.ColorNamePrimary))
}

// stateText is VirtualBox's wording for a VM state.
func stateText(s core.VMState) string {
	if s == core.Stopped {
		return "Powered Off"
	}
	return s.String()
}

// vmList is the left pane, like VirtualBox's machine list: the version icon, the name in bold
// and the state with its symbol.
func (u *ui) vmList() fyne.CanvasObject {
	u.vmItem = widget.NewList(
		func() int { return len(u.vms) },
		func() fyne.CanvasObject {
			img := canvas.NewImageFromResource(versionIcon(u.cfg.Version))
			img.FillMode = canvas.ImageFillContain
			img.SetMinSize(fyne.NewSquareSize(32))
			name := widget.NewRichText(&widget.TextSegment{Style: widget.RichTextStyle{Inline: true, TextStyle: fyne.TextStyle{Bold: true}}},
				&widget.TextSegment{Style: widget.RichTextStyleInline})
			name.Truncation = fyne.TextTruncateEllipsis
			state := widget.NewLabel("")
			state.Truncation = fyne.TextTruncateEllipsis
			icon := widget.NewIcon(stateIcon(core.Stopped))
			stateRow := container.NewBorder(nil, nil, icon, nil, state)
			text := container.New(layout.NewCustomPaddedVBoxLayout(-12), layout.NewSpacer(), name, stateRow, layout.NewSpacer())
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
			name := box.Objects[1].(*widget.RichText)
			stateRow := box.Objects[2].(*fyne.Container)
			state, icon := stateRow.Objects[0].(*widget.Label), stateRow.Objects[1].(*widget.Icon)
			if r := versionIcon(e.Config.Version); img.Resource != r {
				img.Resource = r
				img.Refresh()
			}
			bold, os := name.Segments[0].(*widget.TextSegment), name.Segments[1].(*widget.TextSegment)
			if n, o := e.DisplayName(), " ("+strings.TrimPrefix(versionTitle(e.Config.Version), "macOS ")+")"; bold.Text != n || os.Text != o {
				bold.Text, os.Text = n, o
				name.Refresh()
			}
			s := core.Stopped
			if e.ID == u.runningID {
				s = u.vm.State()
			}
			t, r := stateText(s), stateIcon(s)
			if s == core.Stopped {
				if _, ok := core.FindRunning(e.Config); ok { // e.g. started by OSX-KVM's script
					t, r = "Running (outside the app)", stateIcon(core.Running)
				}
			}
			if state.Text != t {
				state.SetText(t)
			}
			if icon.Resource != r {
				icon.SetResource(r)
			}
		},
	)
	u.vmItem.OnSelected = u.selectVM
	u.selectRow()
	width := canvas.NewRectangle(color.Transparent) // keeps the pane wide enough for the name and state
	width.SetMinSize(fyne.NewSize(200, 0))
	return container.NewStack(width, themedRect(colorListBackground), u.vmItem)
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

// detailsTab mirrors VirtualBox's Details pane: stacked sections, each a tinted band with an
// icon and a title over compact "Key:  value" lines, with the Preview beside General and System.
func (u *ui) detailsTab() fyne.CanvasObject {
	u.details = map[string]*canvas.Text{}
	add := func(title, icon string, fallback fyne.Resource, keys ...string) fyne.CanvasObject {
		box, values := infoSection(title, appIcon(icon, fallback), keys...)
		for k, v := range values {
			u.details[title+"/"+k] = v
		}
		return box
	}
	general := add("General", "general", theme.InfoIcon(), "Name", "Operating System", "Mac Model")
	system := add("System", "system", theme.ComputerIcon(), "Base Memory", "Processors", "CPU Model", "Firmware", "Acceleration")

	// Preview: a framed 16:9 screen; the live picture while running, the VM's name when not.
	u.preview = canvas.NewImageFromImage(image.NewRGBA(image.Rect(0, 0, 1, 1)))
	u.preview.FillMode = canvas.ImageFillContain
	u.preview.ScaleMode = canvas.ImageScaleFastest
	u.preview.SetMinSize(fyne.NewSize(220, 124)) // with its frame, 242 px wide like VirtualBox's
	u.previewBg = canvas.NewRectangle(color.Black)
	u.previewBg.SetMinSize(fyne.NewSize(220, 124))
	u.previewName = canvas.NewText("", color.White)
	u.previewName.TextStyle = fyne.TextStyle{Bold: true}
	u.previewName.TextSize = theme.TextSubHeadingSize()
	u.previewIdle = container.NewCenter(u.previewName)
	frame := themedRect(theme.ColorNameShadow)
	screen := container.NewStack(u.previewBg, u.preview, u.previewIdle)
	previewBox := container.NewVBox(sectionBand("Preview", appIcon("preview", theme.MediaPhotoIcon())),
		container.NewPadded(container.NewStack(frame, container.NewPadded(screen))))

	top := container.NewBorder(nil, nil, nil, previewBox, container.NewVBox(general, system))
	rest := container.NewVBox(
		add("Display", "display", theme.ViewFullScreenIcon(), "Shown", "Graphics Controller", "Resolution", "Remote Display"),
		add("Storage", "storage", theme.StorageIcon(), "Controller", "SATA Port 0", "SATA Port 1", "Boot Disk"),
		add("Audio", "audio", theme.VolumeUpIcon(), "Controller"),
		add("Network", "network", theme.MailSendIcon(), "Adapter 1", "Adapter 2", "Adapter 3", "Adapter 4", "Port Forwarding"),
		add("USB", "usb", theme.SettingsIcon(), "Controller", "Devices"),
		add("GPU Passthrough", "gpu", theme.ColorPaletteIcon(), "GPU", "3D Acceleration"),
	)
	return container.NewVScroll(container.NewVBox(top, rest))
}

func (u *ui) refreshDetails() {
	set := func(k, v string) {
		t, key := u.details[k], u.details[k+"\x00key"]
		if t == nil {
			return
		}
		if t.Text != v || t.Hidden != (v == "") {
			t.Text = v
			t.Hidden, key.Hidden = v == "", v == "" // an empty value hides its row
			t.Refresh()
			key.Refresh()
		}
	}
	p := core.ProfileFor(u.cfg.Version)
	state := u.vm.State()
	os := versionTitle(u.cfg.Version)
	if p.Experimental {
		os += " (experimental)"
	}
	set("General/Name", u.vmName())
	set("General/Operating System", os)
	set("General/Mac Model", core.SMBIOSModel(u.cfg.Version)+" (SMBIOS)")
	set("System/Base Memory", fmt.Sprintf("%d MB", u.cfg.RAMMiB))
	set("System/Processors", fmt.Sprintf("%d (%d cores × %d threads)", u.cfg.VCPUs, u.cfg.Cores, u.cfg.VCPUs/max(u.cfg.Cores, 1)))
	set("System/CPU Model", strings.SplitN(p.CPU, ",", 2)[0])
	set("System/Firmware", "UEFI (OVMF) with OpenCore")
	set("System/Acceleration", "KVM, Q35 chipset")
	set("Display/Shown", map[bool]string{true: "In the app", false: "QEMU window"}[u.cfg.InApp()])
	set("Display/Graphics Controller", "VMware SVGA")
	set("Display/Resolution", "1920x1080")
	set("Display/Remote Display", map[bool]string{true: "VNC on a private socket", false: "Disabled"}[u.cfg.InApp()])
	adapters := u.cfg.Adapters()
	for i := 0; i < core.MaxNICs; i++ {
		v := ""
		if i < len(adapters) && adapters[i].Enabled {
			v = adapterSummary(adapters[i])
		}
		set(fmt.Sprintf("Network/Adapter %d", i+1), v)
	}
	fwd := "None: use the VM's address on its network, if reachable"
	if u.cfg.SSHForwarded() {
		fwd = fmt.Sprintf("SSH 127.0.0.1:%d → 22", u.cfg.SSHPort)
	}
	set("Network/Port Forwarding", fwd)
	set("USB/Devices", "Keyboard, Tablet")
	gpu, accel := "None", "Disabled: the CPU draws the screen"
	if u.cfg.GPU != "" {
		gpu, accel = u.cfg.GPU+" (vfio-pci)", "Metal through the passed-through GPU"
	}
	set("GPU Passthrough/GPU", gpu)
	set("GPU Passthrough/3D Acceleration", accel)
	disk, media := "mac_hdd_ng.img (not created yet)", "Empty"
	if w, err := u.workspace(); err == nil {
		if w.HasDisk() {
			disk = fmt.Sprintf("mac_hdd_ng.img (Normal, %.2f GB, %.2f GB used)",
				float64(core.DiskVirtualSize(w.Path(core.DiskFile)))/(1<<30), float64(core.Allocated(w.Path(core.DiskFile)))/(1<<30))
		}
		if w.HasMedia() {
			media = "BaseSystem.img (" + w.MediaVersion() + " recovery)"
		}
	}
	set("Storage/SATA Port 0", disk)
	set("Storage/SATA Port 1", media)
	set("Storage/Boot Disk", "OpenCore (built in, read-only)")
	set("Storage/Controller", "SATA (ICH9 AHCI)")
	set("Audio/Controller", "Intel HD Audio")
	set("USB/Controller", "xHCI (USB 3.0)")

	if u.previewName.Text != u.vmName() {
		u.previewName.Text = u.vmName()
		u.previewName.Refresh()
	}
	if state == core.Stopped { // the live preview ends with the VM
		u.preview.Hide()
		u.previewBg.Show()
		u.previewIdle.Show()
	}
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
	text := "● " + stateText(s)
	switch {
	case s != core.Stopped && u.cfg.InApp():
		text += "  ·  Click the screen to type; keys go to macOS while it has focus. Ctrl+Alt+F leaves full screen."
	case s != core.Stopped:
		text += "  ·  In the QEMU window."
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
		fyne.DoAndWait(func() { u.screen.attach(v) }) // OnUpdate is set before frames arrive
		if err := v.Run(); err != nil && u.vm.State() != core.Stopped {
			u.logAsync("screen: " + err.Error())
		}
		v.Close()
		fyne.Do(func() { u.screen.detach(v) })
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

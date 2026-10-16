package gui

import (
	"context"
	"errors"
	"fmt"
	"image/color"
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

// The Install, Host and GPU tabs share one look: a header band, then cards on a lighter surface.
// Each item card has a square badge (a number or an icon) whose color tells its state, a title,
// a short description, an optional detail line and a state label on the right; the item that
// needs attention gets an accent stripe.

// itemCard is one row of those tabs. Its colors are applied on every mark, so cards that are
// rebuilt on refresh never register theme callbacks.
type itemCard struct {
	bg, badge, stripe *canvas.Rectangle
	number            *canvas.Text
	icon              *canvas.Image
	state, detail     *widget.Label
	body              *fyne.Container // the item's controls; hidden once a step is done
	obj               fyne.CanvasObject
}

func newItemCard(badgeText, title, desc string, body ...fyne.CanvasObject) *itemCard {
	c := &itemCard{}
	c.badge = canvas.NewRectangle(color.Transparent)
	c.badge.SetMinSize(fyne.NewSquareSize(30))
	c.number = canvas.NewText(badgeText, color.White)
	c.number.TextStyle = fyne.TextStyle{Bold: true}
	c.number.Alignment = fyne.TextAlignCenter
	c.icon = canvas.NewImageFromResource(nil)
	c.icon.FillMode = canvas.ImageFillContain
	c.icon.SetMinSize(fyne.NewSquareSize(18))
	badge := container.NewStack(c.badge, container.NewCenter(c.number), container.NewCenter(c.icon))

	t := wrapped(title)
	t.TextStyle = fyne.TextStyle{Bold: true}
	c.state = widget.NewLabel("")
	c.state.TextStyle = fyne.TextStyle{Bold: true}
	parts := []fyne.CanvasObject{t}
	if desc != "" {
		d := wrapped(desc)
		d.SizeName = theme.SizeNameCaptionText
		parts = append(parts, d)
	}
	c.detail = wrapped("")
	c.detail.Hide()
	parts = append(parts, c.detail)
	text := container.New(layout.NewCustomPaddedVBoxLayout(-8), parts...)
	head := container.NewBorder(nil, nil, container.NewVBox(badge), container.NewVBox(c.state), text)
	c.body = container.NewVBox(body...)
	content := container.NewVBox(head, c.body)

	c.stripe = canvas.NewRectangle(color.Transparent)
	c.stripe.SetMinSize(fyne.NewSize(4, 0))
	c.bg = canvas.NewRectangle(color.Transparent)
	c.obj = container.NewStack(c.bg, container.NewBorder(nil, nil, c.stripe, nil, container.NewPadded(content)))
	return c
}

// mark sets the badge color and icon (nil shows the badge text), the state label, the detail
// line and whether the accent stripe shows.
func (c *itemCard) mark(badge fyne.ThemeColorName, icon fyne.Resource, state string, imp widget.Importance, detail string, accent bool) {
	c.bg.FillColor = theme.Color(theme.ColorNameInputBackground)
	c.badge.FillColor = theme.Color(badge)
	c.number.Hidden = icon != nil
	c.icon.Hidden = icon == nil
	if icon != nil {
		c.icon.Resource = theme.NewColoredResource(icon, theme.ColorNameForegroundOnSuccess)
	}
	c.stripe.FillColor = color.Transparent
	if accent {
		c.stripe.FillColor = theme.Color(theme.ColorNamePrimary)
	}
	c.state.Text, c.state.Importance = state, imp
	c.detail.Text, c.detail.Hidden = detail, detail == ""
	for _, o := range []fyne.CanvasObject{c.bg, c.badge, c.stripe, c.number, c.icon, c.state, c.detail} {
		o.Refresh()
	}
}

// progressLine is a thin progress bar (track and fill), like a modern installer's.
type progressLine struct {
	track, fill *canvas.Rectangle
	value       float64
	obj         *fyne.Container
}

func newProgressLine() *progressLine {
	p := &progressLine{track: canvas.NewRectangle(color.Transparent), fill: canvas.NewRectangle(color.Transparent)}
	p.obj = container.New(p, p.track, p.fill)
	return p
}

func (p *progressLine) set(v float64) {
	p.value = min(max(v, 0), 1)
	p.track.FillColor = theme.Color(theme.ColorNameDisabledButton)
	p.fill.FillColor = theme.Color(theme.ColorNameSuccess)
	p.track.Refresh()
	p.fill.Refresh()
	p.obj.Refresh()
}

func (p *progressLine) MinSize([]fyne.CanvasObject) fyne.Size { return fyne.NewSize(40, 6) }

func (p *progressLine) Layout(_ []fyne.CanvasObject, size fyne.Size) {
	p.track.Resize(size)
	p.fill.Resize(fyne.NewSize(size.Width*float32(p.value), size.Height))
}

// tabHeader is the band at the top of the Install, Host and GPU tabs, with its own content.
func tabHeader(title, icon string, fallback fyne.Resource, content ...fyne.CanvasObject) fyne.CanvasObject {
	return container.NewVBox(sectionBand(title, appIcon(icon, fallback)), container.NewPadded(container.NewVBox(content...)))
}

// surface is a static card on the lighter surface, for notes and controls.
func surface(content ...fyne.CanvasObject) fyne.CanvasObject {
	return container.NewStack(themedRect(theme.ColorNameInputBackground), container.NewPadded(container.NewVBox(content...)))
}

// tabPage stacks a tab's header and cards with a little room around them.
func tabPage(objs ...fyne.CanvasObject) fyne.CanvasObject {
	return container.NewVScroll(container.NewPadded(container.NewVBox(objs...)))
}

type stepState int

const (
	stepWaiting stepState = iota
	stepNext
	stepDone
)

// markStep shows an Install step as done (green check), next (blue, with the stripe) or waiting.
func markStep(c *itemCard, st stepState, detail string) {
	switch st {
	case stepDone:
		c.mark(theme.ColorNameSuccess, theme.ConfirmIcon(), "Done", widget.SuccessImportance, detail, false)
	case stepNext:
		c.mark(theme.ColorNamePrimary, nil, "Next", widget.HighImportance, detail, true)
	default:
		c.mark(theme.ColorNameDisabled, nil, "Waiting", widget.LowImportance, detail, false)
	}
}

func (u *ui) installTab() fyne.CanvasObject {
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
			dialog.ShowError(errors.New("disk size: enter a whole number of GB"), u.win)
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
					dialog.ShowInformation("Install media removed", fmt.Sprintf("%.2f GB freed.", float64(freed)/(1<<30)), u.win)
				})
			}, u.win)
	})
	// The full installer (Big Sur and later): downloaded once into the shared cache as a disc that
	// the VM gets with its install media, so the recovery installs with no network.
	u.btnFullInstaller = widget.NewButtonWithIcon("Download full installer", theme.DownloadIcon(), func() {
		version := u.cfg.Version
		u.run("Download full installer ("+version+")", func() error {
			_, err := core.DownloadInstaller(context.Background(), version, func(p core.Progress) {
				fyne.Do(func() { u.showProgress(p) })
			}, u.logAsync)
			return err
		}, nil)
	})
	u.btnDeleteInstaller = widget.NewButtonWithIcon("Delete installer", theme.DeleteIcon(), func() {
		ci, ok := core.CachedInstallerFor(u.cfg.Version)
		if !ok {
			return
		}
		dialog.ShowConfirm("Delete full installer", fmt.Sprintf("Delete the cached macOS %s installer (%.1f GB)? Every VM of this version uses it.", ci.Version, float64(ci.Size)/1e9),
			func(yes bool) {
				if yes {
					if err := core.RemoveInstaller(ci.Build); err != nil {
						dialog.ShowError(err, u.win)
					}
					u.refresh()
				}
			}, u.win)
	})
	u.installMode = widget.NewLabelWithStyle("", fyne.TextAlignLeading, fyne.TextStyle{Bold: true})
	u.installMode.Wrapping = fyne.TextWrapWord
	u.installerInfo = caption("")
	u.installerInfo.Wrapping = fyne.TextWrapWord
	u.installerCmd = widget.NewEntry()
	unattended := widget.NewCheck("Unattended: erase the VM's empty disk and accept Apple's macOS license for me", func(on bool) {
		u.installerCmd.SetText(core.DiscCommand + map[bool]string{true: " -u"}[on])
	})
	u.installerCmd.SetText(core.DiscCommand)
	copyCmd := widget.NewButtonWithIcon("Copy", theme.ContentCopyIcon(), func() {
		fyne.CurrentApp().Clipboard().SetContent(u.installerCmd.Text)
	})
	typeCmd := widget.NewButtonWithIcon("Type it in the VM", theme.MailForwardIcon(), func() {
		line := core.DiscTyped + map[bool]string{true: " -u"}[unattended.Checked]
		if err := u.screen.typeLine(line); err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		u.tabs.SelectIndex(1) // watch it on the Screen tab
	})
	typeCmd.Importance = widget.HighImportance
	cmdNote := caption("The installer is attached to the VM as the disc OFFLINE, so no network is needed. When the recovery shows its window, click Type it in the VM (with Caps Lock off): " +
		"the app opens Utilities › Terminal and types the line. Without Unattended, first erase the disk in Disk Utility (APFS, named \"Macintosh HD\") and quit it; " +
		"the line copies the installer onto that disk and opens it. Unattended erases the empty disk itself and installs with no questions: macOS restarts by itself and is ready at Setup Assistant.")
	cmdNote.Wrapping = fyne.TextWrapWord
	u.installerCmdBox = container.NewVBox(widget.NewSeparator(), widget.NewLabelWithStyle("Install from the full installer", fyne.TextAlignLeading, fyne.TextStyle{Bold: true}),
		unattended, container.NewBorder(nil, nil, nil, container.NewHBox(copyCmd, typeCmd), u.installerCmd), cmdNote)
	u.installerCmdBox.Hide()

	startVM := widget.NewButtonWithIcon("Start the VM", theme.MediaPlayIcon(), func() { u.btnStart.trigger() })
	u.btnStartInstall = startVM
	// The same start with macOS's boot log on screen, for an install stuck at the Apple logo.
	u.btnVerboseInstall = widget.NewButtonWithIcon("Start with verbose boot", theme.ListIcon(), func() {
		if !u.btnStart.Disabled() {
			u.startWith(true)
		}
	})
	debugNote := caption("Stuck at the Apple logo? Start with verbose boot shows macOS's boot log instead of the logo, for that start only. " +
		"A log that ends at EXITBS:START usually means a core count other than 1, 2, 4, 8 or 16 (Settings › System › Processor).")
	debugNote.Wrapping = fyne.TextWrapWord

	right := func(b fyne.CanvasObject) fyne.CanvasObject { return container.NewHBox(layout.NewSpacer(), b) }
	sizeBox := container.NewGridWrap(fyne.NewSize(90, u.diskSize.MinSize().Height), u.diskSize)
	diskRow := container.NewHBox(widget.NewLabel("Size"), sizeBox, widget.NewLabel("GB (thin, at least 40)"), layout.NewSpacer(), u.btnDisk)
	guide := widget.NewAccordion(widget.NewAccordionItem("Show the steps inside macOS", markdown(manualSteps)))

	var cards []fyne.CanvasObject
	add := func(n int, title, desc string, body ...fyne.CanvasObject) *itemCard {
		c := newItemCard(strconv.Itoa(n), title, desc, body...)
		cards = append(cards, c.obj)
		return c
	}
	u.steps = []*itemCard{
		add(1, "VM folder", "Creates the VM's folder and extracts the built-in OpenCore and OVMF.", right(u.btnPrepare)),
		add(2, "macOS recovery", "Downloaded from Apple; the signature and every chunk's SHA-256 are verified. "+
			"Optionally also download the full installer (12–18 GB) once for every VM of this version, instead of letting each install download macOS again.",
			u.stage, u.bar, u.barInf, u.installerInfo, right(container.NewHBox(u.btnDeleteInstaller, u.btnFullInstaller, u.btnDownload))),
		add(3, "Virtual hard disk", "A qcow2 disk that only takes the space macOS uses.", diskRow),
		add(4, "Install macOS", "Start the VM, erase the disk in Disk Utility and run Reinstall macOS, or install from the full installer.", u.installMode, guide, u.installerCmdBox, debugNote,
			right(container.NewHBox(u.btnVerboseInstall, startVM))),
		add(5, "Clean up", "Once macOS boots from its disk, remove the install media to free space.", right(u.btnRemove)),
	}

	u.installTitle = widget.NewLabel("")
	u.installTitle.TextStyle = fyne.TextStyle{Bold: true}
	u.installBar = newProgressLine()
	u.installFiles = caption("")
	u.installFiles.Wrapping = fyne.TextWrapWord
	header := tabHeader("Installation", "storage", theme.StorageIcon(), u.installTitle, u.installBar.obj, u.installFiles)
	return tabPage(append([]fyne.CanvasObject{header}, cards...)...)
}

// refreshInstall works out each step's state from the files in the VM's folder.
func (u *ui) refreshInstall(w core.Workspace, wsErr error, ready, free bool) {
	if u.steps == nil {
		return
	}
	hasDisk, hasMedia := wsErr == nil && w.HasDisk(), wsErr == nil && w.HasMedia()
	var used float64
	if hasDisk {
		used = float64(core.Allocated(w.Path(core.DiskFile))) / (1 << 30)
	}
	// Without media but with a disk holding a system, macOS was installed and cleaned up.
	installed := hasDisk && !hasMedia && used > 10

	done := []bool{ready, hasMedia || installed, hasDisk, installed, installed}
	details := []string{"Not created yet", "Not downloaded yet", "Not created yet", "", ""}
	if wsErr != nil {
		details[0] = "Invalid folder: " + wsErr.Error()
	} else if ready {
		details[0] = w.Dir
	}
	if hasMedia {
		details[1] = fmt.Sprintf("BaseSystem.img · %s · %.2f GB", w.MediaVersion(), float64(core.Allocated(w.Path(core.MediaFile)))/(1<<30))
	} else if installed {
		details[1] = "Removed after the installation"
	}
	if hasDisk {
		details[2] = fmt.Sprintf("mac_hdd_ng.img · %.2f GB used on the host", used)
	}
	if installed {
		details[3] = "macOS is installed on the disk"
		details[4] = "The install media is gone"
	}
	next, count := -1, 0
	for i, d := range done {
		if d {
			count++
		} else if next < 0 {
			next = i
		}
	}
	for i, s := range u.steps {
		st := stepWaiting
		switch {
		case done[i]:
			st = stepDone
		case i == next:
			st = stepNext
		}
		markStep(s, st, details[i])
		// The next step's action is the highlighted one, like the default button of an installer.
		if b := []*widget.Button{u.btnPrepare, u.btnDownload, u.btnDisk, u.btnStartInstall, u.btnRemove}[i]; b != nil {
			b.Importance = map[bool]widget.Importance{true: widget.HighImportance, false: widget.MediumImportance}[st == stepNext]
			b.Refresh()
		}
		// The recovery's step stays open: the full installer is optional and usually comes after it.
		s.body.Hidden = st == stepDone && !(i == 1 && !installed && core.InstallerSupported(u.cfg.Version))
		s.body.Refresh()
	}
	u.stage.Hidden = u.stage.Text == ""
	if count == len(done) {
		u.installTitle.SetText("macOS is installed: start the VM from the toolbar")
	} else {
		u.installTitle.SetText(fmt.Sprintf("%d of %d steps done", count, len(done)))
	}
	u.installBar.set(float64(count) / float64(len(done)))
	u.installFiles.SetText("Boot files: " + core.AssetVersions())

	enable := func(b *widget.Button, on bool) {
		if on {
			b.Enable()
		} else {
			b.Disable()
		}
	}
	enable(u.btnPrepare, free && wsErr == nil && !ready)
	enable(u.btnDownload, free && wsErr == nil && !installed)
	ci, cached := core.CachedInstallerFor(u.cfg.Version)
	switch {
	case !core.InstallerSupported(u.cfg.Version):
		u.installerInfo.SetText("Full installer: Apple has none for this macOS version; the recovery installs it.")
	case cached:
		u.installerInfo.SetText(fmt.Sprintf("Full installer: macOS %s (%s), %.1f GB, cached for every VM of this version.", ci.Version, ci.Build, float64(ci.Size)/1e9))
	default:
		u.installerInfo.SetText("Full installer: not downloaded (optional).")
	}
	runningHere := u.runningID == u.cur.ID && u.vm.State() != core.Stopped
	switch {
	case cached && runningHere && u.vm.InstallerCommand() == "":
		u.installMode.SetText("Install mode: Offline, once the VM restarts: it was started before the installer was cached, so it has no OFFLINE disc yet.")
	case cached:
		u.installMode.SetText(fmt.Sprintf("Install mode: Offline. The VM starts with the disc OFFLINE (macOS %s) and needs no network; use the box below.", ci.Version))
	case core.InstallerSupported(u.cfg.Version):
		u.installMode.SetText("Install mode: Online. Reinstall macOS downloads macOS (about 13 GB) inside the VM. To install offline, download the full installer in step 2 first.")
	default:
		u.installMode.SetText("Install mode: Online. Reinstall macOS downloads macOS inside the VM (Apple has no full installer for this version).")
	}
	enable(u.btnFullInstaller, free && core.InstallerSupported(u.cfg.Version) && !installed)
	enable(u.btnDeleteInstaller, free && cached)
	if u.vm.InstallerCommand() != "" && u.runningID == u.cur.ID {
		u.installerCmdBox.Show()
	} else {
		u.installerCmdBox.Hide()
	}
	enable(u.btnDisk, free && ready && !hasDisk)
	enable(u.btnRemove, free && hasMedia)
	enable(u.btnStartInstall, free && ready && hasDisk && hasMedia)
	enable(u.btnVerboseInstall, free && ready && hasDisk)
	u.diskSize.Hidden = hasDisk
}

func (u *ui) showInfinite(stage string) {
	u.stage.SetText(stage)
	u.stage.Show()
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
	u.stage.Show()
	u.barInf.Stop()
	u.barInf.Hide()
	u.bar.Show()
	u.bar.SetValue(p.Pct / 100)
}

// offlineInstallersDialog lists every cached full installer (the offline install media), with
// the VMs whose macOS version uses it, and deletes them.
func (u *ui) offlineInstallersDialog() {
	var d dialog.Dialog
	list := container.NewVBox()
	cached := core.CachedInstallers()
	vms, _ := core.ListVMs()
	for _, c := range cached {
		var users []string
		for _, e := range vms {
			if ci, ok := core.CachedInstallerFor(e.Config.Version); ok && ci.Build == c.Build {
				users = append(users, e.DisplayName())
			}
		}
		use := "no VM of this version"
		if len(users) > 0 {
			use = "used by " + strings.Join(users, ", ")
		}
		del := widget.NewButtonWithIcon("Delete", theme.DeleteIcon(), func() {
			dialog.ShowConfirm("Delete offline installer", fmt.Sprintf("Delete the macOS %s installer (%.1f GB)? VMs of this version then install online, or download it again.", c.Version, float64(c.Size)/1e9),
				func(yes bool) {
					if !yes {
						return
					}
					if err := core.RemoveInstaller(c.Build); err != nil {
						dialog.ShowError(err, u.win)
						return
					}
					d.Hide()
					u.refresh()
					u.offlineInstallersDialog()
				}, u.win)
		})
		info := widget.NewLabel(fmt.Sprintf("macOS %s (%s) · %.1f GB · %s", c.Version, c.Build, float64(c.Size)/1e9, use))
		info.Wrapping = fyne.TextWrapWord
		list.Add(container.NewBorder(nil, nil, nil, del, info))
	}
	if len(cached) == 0 {
		list.Add(note("No offline installers yet. Download one with Download full installer in a VM's Install step 2 (Big Sur and later); every VM of that macOS version then installs from it with no network."))
	}
	dir, _ := core.InstallersDir()
	content := container.NewVBox(list, widget.NewSeparator(), note("Kept in "+dir+", one per macOS build, shared by every VM of that version."))
	d = dialog.NewCustom("Offline installers", "Close", container.NewPadded(content), u.win)
	d.Resize(fyne.NewSize(640, 0))
	d.Show()
}

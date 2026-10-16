// macos-kvm-gui: desktop app to install and run macOS on KVM using OSX-KVM.
package main

import (
	"context"
	"errors"
	"fmt"
	"runtime"
	"strconv"
	"strings"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/app"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/dialog"
	"fyne.io/fyne/v2/theme"
	"fyne.io/fyne/v2/widget"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

const manualSteps = `**Manual steps inside the VM (first install)**

1. In the OpenCore menu, choose **macOS Base System**.
2. Open **Disk Utility**, enable *View › Show All Devices*, select the **QEMU HARDDISK** disk and use **Erase** (APFS, GUID).
3. Close Disk Utility and choose **Reinstall macOS**. The VM restarts a few times: in OpenCore pick **macOS Installer**, then the installed disk.
4. Once macOS boots from the disk, remove the install media here to free space.`

const videoTips = `**Video performance**

The VM has no graphics acceleration: macOS has no driver for the emulated display or for NVIDIA GPUs, so the CPU draws the screen. To make it smoother:

1. Use **Recommended for this host** in the Settings tab (more vCPUs and RAM).
2. In macOS, under *System Settings › Accessibility › Display*, turn on **Reduce motion** and **Reduce transparency**.
3. Under *Desktop & Dock*, minimize with **Scale** and turn off **Animate opening applications**.
4. Use a static wallpaper (the default Sonoma wallpaper is animated and stays blank without Metal) and keep 1920x1080.

Real acceleration needs a macOS-supported AMD GPU passed through with VFIO: see the GPU tab.`

const gpuNotes = `**GPU passthrough (VFIO)**

Gives the VM a real AMD GPU, the only way to get Metal. The GPU's display output then goes to a monitor plugged into that card; the QEMU window stays for keyboard and mouse.

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

	checks      *fyne.Container
	installInfo *widget.Label
	stage       *widget.Label
	bar         *widget.ProgressBar
	barInf      *widget.ProgressBarInfinite
	diskSize    *widget.Entry
	vmState     *widget.Label
	log         *widget.TextGrid
	logScroll   *container.Scroll
	logLines    []string
	gpuList     *fyne.Container
	gpuSelect   *widget.Select
	gpuStatus   *widget.Label
	gpus        []core.GPU

	btnPrepare, btnDownload, btnDisk, btnRemove *widget.Button
	btnStart, btnPowerdown, btnKill             *widget.Button
	btnReserve, btnRelease                      *widget.Button
}

func main() {
	a := app.NewWithID("io.github.jniltinho.macos-kvm-gui")
	u := &ui{win: a.NewWindow("macOS KVM"), host: core.SystemHost(), vm: &core.VM{}}
	u.cfgPath, _ = core.ConfigPath()
	cfg, err := core.LoadConfig(u.cfgPath)
	u.cfg = cfg

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

	tabs := container.NewAppTabs(
		container.NewTabItemWithIcon("Host", theme.ComputerIcon(), u.hostTab()),
		container.NewTabItemWithIcon("Settings", theme.SettingsIcon(), u.configTab()),
		container.NewTabItemWithIcon("Install", theme.DownloadIcon(), u.installTab()),
		container.NewTabItemWithIcon("VM", theme.MediaPlayIcon(), u.vmTab()),
		container.NewTabItemWithIcon("GPU", theme.ColorPaletteIcon(), u.gpuTab()),
		container.NewTabItemWithIcon("Log", theme.ListIcon(), u.logTab()),
	)
	u.win.SetContent(tabs)
	u.win.Resize(fyne.NewSize(860, 620))
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
	if err := core.SaveConfig(u.cfgPath, c); err != nil {
		dialog.ShowError(err, u.win)
		return false
	}
	u.cfg = c
	u.refresh()
	return true
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
	fix := widget.NewButtonWithIcon("Fix ignore_msrs", theme.SettingsIcon(), func() {
		go func() {
			err := u.host.FixIgnoreMSRs()
			fyne.Do(func() {
				switch {
				case errors.Is(err, core.ErrCancelled):
					dialog.ShowInformation("ignore_msrs", "Cancelled: nothing was changed.", u.win)
				case err != nil:
					dialog.ShowError(err, u.win)
				default:
					dialog.ShowInformation("ignore_msrs", "ignore_msrs enabled and persisted in /etc/modprobe.d/macos-kvm-gui.conf.", u.win)
				}
				u.refreshChecks()
			})
		}()
	})
	return container.NewBorder(nil, container.NewHBox(recheck, fix), nil, nil, container.NewVScroll(u.checks))
}

func (u *ui) refreshChecks() {
	u.checks.RemoveAll()
	for _, c := range u.host.Checks(u.cfg.Version) {
		text := c.Name + ": " + c.Detail
		if c.Hint != "" {
			text += "\n→ " + c.Hint
		}
		l := widget.NewLabel(text)
		l.Wrapping = fyne.TextWrapWord
		u.checks.Add(container.NewBorder(nil, nil, widget.NewIcon(statusIcon(c.Status)), nil, l))
	}
}

func versionLabel(v core.Version) string {
	if core.ProfileFor(v.Short).Experimental {
		return v.Name + " (experimental)"
	}
	return v.Name
}

func (u *ui) configTab() fyne.CanvasObject {
	ws := widget.NewEntry()
	ws.SetText(u.cfg.Workspace)
	var labels []string
	short := map[string]string{}
	for _, v := range core.Versions {
		l := versionLabel(v)
		labels = append(labels, l)
		short[l] = v.Short
	}
	version := widget.NewSelect(labels, nil)
	for _, v := range core.Versions {
		if v.Short == u.cfg.Version {
			version.SetSelected(versionLabel(v))
		}
	}
	num := func(n int) *widget.Entry { e := widget.NewEntry(); e.SetText(strconv.Itoa(n)); return e }
	ram, cores, vcpus, port := num(u.cfg.RAMMiB), num(u.cfg.Cores), num(u.cfg.VCPUs), num(u.cfg.SSHPort)

	form := widget.NewForm(
		widget.NewFormItem("Workspace", ws),
		widget.NewFormItem("macOS version", version),
		widget.NewFormItem("RAM (MiB)", ram),
		widget.NewFormItem("Cores", cores),
		widget.NewFormItem("vCPUs (total)", vcpus),
		widget.NewFormItem("SSH port (127.0.0.1)", port),
	)
	form.SubmitText = "Save"
	form.OnSubmit = func() {
		if u.vm.State() != core.Stopped {
			dialog.ShowInformation("Settings", "Shut down the VM before changing the settings.", u.win)
			return
		}
		c := u.cfg // keeps fields edited elsewhere, such as the GPU
		c.Workspace, c.Version = strings.TrimSpace(ws.Text), short[version.Selected]
		var bad string
		for _, f := range []struct {
			name string
			e    *widget.Entry
			dst  *int
		}{{"RAM", ram, &c.RAMMiB}, {"Cores", cores, &c.Cores}, {"vCPUs", vcpus, &c.VCPUs}, {"SSH port", port, &c.SSHPort}} {
			n, err := strconv.Atoi(strings.TrimSpace(f.e.Text))
			if err != nil && bad == "" {
				bad = f.name + ": enter a whole number"
			}
			*f.dst = n
		}
		if bad != "" {
			dialog.ShowError(errors.New(bad), u.win)
			return
		}
		if u.saveConfig(c) {
			dialog.ShowInformation("Settings", "Settings saved.", u.win)
		}
	}
	recommend := widget.NewButtonWithIcon("Recommended for this host", theme.ComputerIcon(), func() {
		r := core.Recommended(u.cfg, runtime.NumCPU(), u.host.MemTotalMiB())
		ram.SetText(strconv.Itoa(r.RAMMiB))
		cores.SetText(strconv.Itoa(r.Cores))
		vcpus.SetText(strconv.Itoa(r.VCPUs))
	})
	note := widget.NewLabel("Without a supported GPU, macOS draws the screen on the CPU: more vCPUs make video smoother. " +
		"The button above only fills in the fields; click Save to apply. " +
		"The CPU model and network card follow the macOS version. Only Sonoma has been validated end to end.")
	note.Wrapping = fyne.TextWrapWord
	return container.NewVBox(form, container.NewHBox(recommend), note)
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

	u.btnPrepare = widget.NewButtonWithIcon("Prepare workspace (clone OSX-KVM)", theme.FolderNewIcon(), func() {
		w, err := u.workspace()
		if err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		u.showInfinite("Cloning OSX-KVM")
		u.run("Prepare workspace", func() error { return w.Prepare(context.Background(), u.logAsync) }, nil)
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

	disk := container.NewBorder(nil, nil, widget.NewLabel("Size (GiB, min. 40):"), u.btnDisk, u.diskSize)
	return container.NewVScroll(container.NewVBox(
		u.installInfo,
		u.btnPrepare,
		u.btnDownload,
		u.stage, u.bar, u.barInf,
		disk,
		u.btnRemove,
		widget.NewSeparator(),
		widget.NewRichTextFromMarkdown(manualSteps),
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

func (u *ui) vmTab() fyne.CanvasObject {
	u.vmState = widget.NewLabel("")
	u.btnStart = widget.NewButtonWithIcon("Start", theme.MediaPlayIcon(), func() {
		if err := u.vm.Start(u.cfg); err != nil {
			dialog.ShowError(err, u.win)
		}
		u.refresh()
	})
	u.btnPowerdown = widget.NewButtonWithIcon("Shut down (ACPI)", theme.LogoutIcon(), func() {
		go func() {
			err := u.vm.Powerdown()
			fyne.Do(func() {
				if err != nil {
					dialog.ShowError(err, u.win)
				} else {
					u.appendLog("[VM] shutdown request sent to macOS")
				}
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
	u.btnPowerdown.Importance = widget.WarningImportance
	u.btnKill.Importance = widget.DangerImportance
	return container.NewVScroll(container.NewVBox(
		u.vmState,
		container.NewHBox(u.btnStart, u.btnPowerdown, u.btnKill),
		widget.NewSeparator(),
		widget.NewRichTextFromMarkdown(videoTips),
	))
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
		u.gpuList,
		widget.NewSeparator(),
		container.NewBorder(nil, nil, widget.NewLabel("GPU for the VM:"), nil, u.gpuSelect),
		container.NewHBox(u.btnReserve, u.btnRelease),
		u.gpuStatus,
		widget.NewSeparator(),
		widget.NewRichTextFromMarkdown(gpuNotes),
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
		icon := theme.ErrorIcon()
		text := fmt.Sprintf("%s  %s [%s]\n%s · driver %s · IOMMU group %s", g.Addr, g.Name, g.ID, g.Compat, g.Driver, g.Group)
		if g.Eligible {
			icon = theme.ConfirmIcon()
			text += "\n→ can be passed to the VM"
		} else {
			text += "\n→ " + g.Reason
		}
		l := widget.NewLabel(text)
		l.Wrapping = fyne.TextWrapWord
		u.gpuList.Add(container.NewBorder(nil, nil, widget.NewIcon(icon), nil, l))
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
	return u.logScroll
}

// refresh recomputes texts and buttons from the real state of the disk and the VM.
func (u *ui) refresh() {
	if u.installInfo == nil || u.gpuList == nil {
		return
	}
	u.refreshChecks()
	u.refreshGPUs()
	state := u.vm.State()
	running := state != core.Stopped
	free := !u.busy && !running

	w, err := u.workspace()
	var lines []string
	st := core.WorkspaceIncomplete
	if err != nil {
		lines = append(lines, "Invalid workspace: "+err.Error())
	} else {
		var missing []string
		st, missing = w.Classify()
		switch st {
		case core.WorkspaceEmpty:
			lines = append(lines, "Workspace "+w.Dir+": empty (click Prepare)")
		case core.WorkspaceReady:
			lines = append(lines, "Workspace "+w.Dir+": ready")
		default:
			lines = append(lines, "Workspace "+w.Dir+": not a complete OSX-KVM; missing "+strings.Join(missing, ", "))
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
	ready := st == core.WorkspaceReady
	enable(u.btnPrepare, free && st == core.WorkspaceEmpty)
	enable(u.btnDownload, free && ready)
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

	p := core.ProfileFor(u.cfg.Version)
	display := "emulated display"
	if u.cfg.GPU != "" {
		display = "GPU " + u.cfg.GPU
	}
	u.vmState.SetText(fmt.Sprintf("State: %s\nmacOS %s · %d MiB · %d vCPUs (%d cores) · CPU %s · %s · SSH: ssh -p %d user@127.0.0.1",
		state, u.cfg.Version, u.cfg.RAMMiB, u.cfg.VCPUs, u.cfg.Cores, strings.SplitN(p.CPU, ",", 2)[0], display, u.cfg.SSHPort))
}

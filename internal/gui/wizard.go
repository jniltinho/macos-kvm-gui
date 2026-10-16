package gui

import (
	"context"
	"errors"
	"fmt"
	"image/color"
	"path/filepath"
	"runtime"
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

// newVMDialog is the New VM wizard in VirtualBox's expert style: an illustration on the left and
// three folding sections (name and operating system, hardware, hard disk). Finish adds the VM,
// makes its folder and disk and, when asked, downloads the recovery.
func (u *ui) newVMDialog() {
	if u.vm.State() != core.Stopped {
		dialog.ShowInformation("New VM", "Shut down the running VM first.", u.win)
		return
	}
	threadsN, hostMem := runtime.NumCPU(), u.host.MemTotalMiB()
	rec := core.Recommended(core.Defaults(), threadsN, hostMem)

	// Name and operating system.
	name := widget.NewEntry()
	name.SetPlaceHolder("for example macOS Tahoe lab")
	folder := widget.NewEntry()
	folder.SetText(core.VMFolder("macOS"))
	folderEdited := false
	var showLocation func()
	folder.OnChanged = func(string) {
		folderEdited = true
		if showLocation != nil {
			showLocation()
		}
	}
	var labels []string
	short := map[string]string{}
	for _, v := range core.Versions {
		labels = append(labels, versionLabel(v))
		short[versionLabel(v)] = v.Short
	}
	art := canvas.NewImageFromResource(versionIcon("sonoma"))
	art.FillMode = canvas.ImageFillContain
	art.SetMinSize(fyne.NewSquareSize(128))
	osIcon := canvas.NewImageFromResource(versionIcon("sonoma"))
	osIcon.FillMode = canvas.ImageFillContain
	osIcon.SetMinSize(fyne.NewSquareSize(36))
	version := widget.NewSelect(labels, func(l string) {
		for _, img := range []*canvas.Image{art, osIcon} {
			img.Resource = versionIcon(short[l])
			img.Refresh()
		}
	})
	version.SetSelected(labels[6]) // Sonoma, the validated version
	name.OnChanged = func(n string) {
		if !folderEdited {
			folder.SetText(core.VMFolder(strings.TrimSpace(n)))
			folderEdited = false
		}
	}
	identity := container.NewVBox(widget.NewForm(
		widget.NewFormItem("VM Name", name),
		widget.NewFormItem("VM Folder", folder),
		widget.NewFormItem("OS Version", container.NewBorder(nil, nil, nil, osIcon, version)),
	), note("Only Sonoma has been validated end to end. Newer versions need more memory and disk."))

	// Hardware, prefilled with what is recommended for this host.
	ramRow, ramValue, ramSet := sliderRow(core.MinRAMMiB, float64(max(hostMem, core.MinRAMMiB*2)), 512, "MB")
	ramSet(rec.RAMMiB)
	cpuRow, cpuValue, cpuSet := sliderRow(1, float64(threadsN), 1, "CPUs")
	cpuSet(rec.VCPUs)
	display := widget.NewSelect([]string{displayAppLabel, displayWindowLabel}, nil)
	display.SetSelected(displayAppLabel)
	hardware := container.NewVBox(widget.NewForm(
		widget.NewFormItem("Base Memory", ramRow),
		widget.NewFormItem("Number of CPUs", cpuRow),
		widget.NewFormItem("Display", display),
	), note(fmt.Sprintf("Recommended for this host: %d MB and %d CPUs. Without a supported GPU the CPU draws the screen, so more CPUs make it smoother.", rec.RAMMiB, rec.VCPUs)))

	// Hard disk.
	diskRow, diskValue, diskSet := sliderRow(core.MinDiskGiB, 2048, 8, "GB")
	diskSet(80)
	download := widget.NewCheck("Download the macOS recovery from Apple now", nil)
	download.SetChecked(true)
	location := widget.NewLabel("")
	location.Truncation = fyne.TextTruncateEllipsis
	showLocation = func() { location.SetText(filepath.Join(strings.TrimSpace(folder.Text), core.DiskFile)) }
	showLocation()
	disk := container.NewVBox(widget.NewForm(
		widget.NewFormItem("Location", location),
		widget.NewFormItem("Disk Size", diskRow),
		widget.NewFormItem("Format", widget.NewLabel("QCOW2 (QEMU Copy-On-Write), dynamically allocated")),
	), download)

	sections := widget.NewAccordion(
		widget.NewAccordionItem("Virtual machine name and operating system", identity),
		widget.NewAccordionItem("Specify virtual hardware", hardware),
		widget.NewAccordionItem("Specify virtual hard disk", disk),
	)
	sections.Open(0)

	// collect validates everything and opens the section with the first problem.
	collect := func() (core.Config, int, error) {
		c := rec
		c.Display = core.DisplayApp
		fail := func(i int, err error) (core.Config, int, error) {
			sections.CloseAll()
			sections.Open(i)
			return c, 0, err
		}
		c.Name, c.Version = strings.TrimSpace(name.Text), short[version.Selected]
		if c.Name == "" {
			return fail(0, errors.New("VM Name: give the VM a name"))
		}
		for _, e := range u.vms {
			if strings.EqualFold(e.Config.Name, c.Name) {
				return fail(0, errors.New("VM Name: a VM with this name already exists"))
			}
		}
		c.Workspace = strings.TrimSpace(folder.Text)
		dir, err := c.WorkspaceDir()
		if err != nil {
			return fail(0, fmt.Errorf("VM Folder: %w", err))
		}
		if err := core.RequireEmpty(dir); err != nil {
			return fail(0, fmt.Errorf("VM Folder: %w", err))
		}
		if c.RAMMiB, err = ramValue(); err != nil {
			return fail(1, fmt.Errorf("Base Memory: %w", err))
		}
		if c.VCPUs, err = cpuValue(); err != nil {
			return fail(1, fmt.Errorf("Number of CPUs: %w", err))
		}
		c.Cores = c.VCPUs // one thread per core unless the count splits evenly into two
		if c.VCPUs%2 == 0 && threadsN > 1 {
			c.Cores = c.VCPUs / 2
		}
		c.Display = map[string]string{displayAppLabel: core.DisplayApp, displayWindowLabel: core.DisplayWindow}[display.Selected]
		if err := c.Validate(); err != nil {
			return fail(1, err)
		}
		size, err := diskValue()
		if err != nil || size < core.MinDiskGiB {
			return fail(2, fmt.Errorf("Disk Size: at least %d GB", core.MinDiskGiB))
		}
		return c, size, nil
	}

	panel := themedRect(theme.ColorNameHeaderBackground)
	left := container.NewStack(panel, container.NewPadded(container.NewVBox(layout.NewSpacer(), art, layout.NewSpacer())))
	sizer := canvas.NewRectangle(color.Transparent)
	sizer.SetMinSize(fyne.NewSize(150, 0))
	finish := widget.NewButtonWithIcon("Finish", theme.ConfirmIcon(), nil)
	finish.Importance = widget.HighImportance
	cancel := widget.NewButtonWithIcon("Cancel", theme.CancelIcon(), nil)
	content := container.NewBorder(nil,
		container.NewVBox(widget.NewSeparator(), container.NewHBox(layout.NewSpacer(), finish, cancel)),
		container.NewStack(sizer, left), nil,
		container.NewVScroll(container.NewPadded(sections)))
	d := dialog.NewCustomWithoutButtons("New Virtual Machine", content, u.win)
	cancel.OnTapped = d.Hide
	finish.OnTapped = func() {
		c, size, err := collect()
		if err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		d.Hide()
		u.createVM(c, size, download.Checked)
	}
	d.Resize(fyne.NewSize(860, 600))
	d.Show()
	u.win.Canvas().Focus(name)
}

// createVM adds the VM and, in the Install tab, makes its folder and disk and downloads the
// recovery when asked.
func (u *ui) createVM(c core.Config, sizeGiB int, download bool) {
	e, err := core.CreateVM(c)
	if err != nil {
		dialog.ShowError(err, u.win)
		return
	}
	core.SelectVM(e.ID)
	u.reloadVMs(e.ID)
	u.tabs.SelectIndex(2) // Install shows the progress
	dir, _ := c.WorkspaceDir()
	w := core.NewWorkspace(dir)
	u.run("Create VM "+c.Name, func() error {
		if err := w.Prepare(c); err != nil {
			return err
		}
		warning, err := w.CreateDisk(context.Background(), sizeGiB)
		if err != nil {
			return err
		}
		if warning != "" {
			u.logAsync("[disk] " + warning)
		}
		if !download {
			return nil
		}
		return w.DownloadMedia(context.Background(), c.Version, func(p core.Progress) {
			fyne.Do(func() { u.showProgress(p) })
		}, u.logAsync)
	}, func() {
		next := "Click Start and install macOS from the recovery (Disk Utility, then Reinstall macOS)."
		if !download {
			next = "Download the macOS recovery in step 2, then click Start."
		}
		dialog.ShowInformation("New VM", c.Name+" is ready. "+next, u.win)
	})
}

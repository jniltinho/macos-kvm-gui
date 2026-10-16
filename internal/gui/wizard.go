package gui

import (
	"context"
	"errors"
	"fmt"
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

// newVMDialog is the New VM wizard: name and macOS version, resources, storage, summary. Create
// adds the VM, makes its folder and disk and, when asked, downloads the recovery.
func (u *ui) newVMDialog() {
	if u.vm.State() != core.Stopped {
		dialog.ShowInformation("New VM", "Shut down the running VM first.", u.win)
		return
	}
	c := core.Recommended(core.Defaults(), runtime.NumCPU(), u.host.MemTotalMiB())
	c.Display = core.DisplayApp

	// Step 1: name and macOS version.
	name := widget.NewEntry()
	name.SetPlaceHolder("for example macOS Tahoe lab")
	var labels []string
	short := map[string]string{}
	for _, v := range core.Versions {
		labels = append(labels, versionLabel(v))
		short[versionLabel(v)] = v.Short
	}
	icon := canvas.NewImageFromResource(versionIcon("sonoma"))
	icon.FillMode = canvas.ImageFillContain
	icon.SetMinSize(fyne.NewSquareSize(96))
	version := widget.NewSelect(labels, func(l string) {
		icon.Resource = versionIcon(short[l])
		icon.Refresh()
	})
	version.SetSelected(labels[6]) // Sonoma, the validated version
	step1 := container.NewBorder(nil, nil, container.NewCenter(icon), nil, container.NewVBox(
		widget.NewForm(widget.NewFormItem("Name", name), widget.NewFormItem("macOS version", version)),
		wrapped("Only Sonoma has been validated end to end. Newer versions need more RAM and disk."),
	))

	// Step 2: resources, filled in with what is recommended for this host.
	ram, cores, vcpus := widget.NewEntry(), widget.NewEntry(), widget.NewEntry()
	ram.SetText(strconv.Itoa(c.RAMMiB))
	cores.SetText(strconv.Itoa(c.Cores))
	vcpus.SetText(strconv.Itoa(c.VCPUs))
	display := widget.NewSelect([]string{displayAppLabel, displayWindowLabel}, nil)
	display.SetSelected(displayAppLabel)
	step2 := container.NewVBox(
		widget.NewForm(
			widget.NewFormItem("RAM (MiB)", ram),
			widget.NewFormItem("Cores", cores),
			widget.NewFormItem("vCPUs (total)", vcpus),
			widget.NewFormItem("Display", display),
		),
		wrapped(fmt.Sprintf("Recommended for this host (%d threads, %d MiB). Without a supported GPU the CPU draws the screen, so more vCPUs make it smoother.",
			runtime.NumCPU(), u.host.MemTotalMiB())),
	)

	// Step 3: storage. The folder follows the name until it is edited.
	folder := widget.NewEntry()
	folderEdited := false
	folder.OnChanged = func(string) { folderEdited = true }
	name.OnChanged = func(n string) {
		if !folderEdited {
			folder.SetText(core.VMFolder(strings.TrimSpace(n)))
			folderEdited = false
		}
	}
	size := widget.NewEntry()
	size.SetText("80")
	download := widget.NewCheck("Download the macOS recovery from Apple now", nil)
	download.SetChecked(true)
	step3 := container.NewVBox(
		widget.NewForm(widget.NewFormItem("Folder", folder), widget.NewFormItem("Disk size (GiB)", size)),
		download,
		wrapped(fmt.Sprintf("The folder holds the disk, the NVRAM and the boot files. The disk is thin: it grows as macOS uses it. At least %d GiB.", core.MinDiskGiB)),
	)

	summary := wrapped("")
	steps := []struct {
		title string
		body  fyne.CanvasObject
	}{
		{"Name and operating system", step1},
		{"Hardware", step2},
		{"Storage", step3},
		{"Summary", summary},
	}

	// check validates step i and fills c; it returns the problem to show, if any.
	var sizeGiB int
	check := func(i int) error {
		switch i {
		case 0:
			c.Name, c.Version = strings.TrimSpace(name.Text), short[version.Selected]
			if c.Name == "" {
				return errors.New("give the VM a name")
			}
			for _, e := range u.vms {
				if strings.EqualFold(e.Config.Name, c.Name) {
					return errors.New("a VM with this name already exists")
				}
			}
			if !folderEdited || folder.Text == "" {
				folder.SetText(core.VMFolder(c.Name))
				folderEdited = false
			}
		case 1:
			for _, n := range []struct {
				label string
				e     *widget.Entry
				dst   *int
			}{{"RAM", ram, &c.RAMMiB}, {"Cores", cores, &c.Cores}, {"vCPUs", vcpus, &c.VCPUs}} {
				v, err := strconv.Atoi(strings.TrimSpace(n.e.Text))
				if err != nil {
					return fmt.Errorf("%s: enter a whole number", n.label)
				}
				*n.dst = v
			}
			c.Display = map[string]string{displayAppLabel: core.DisplayApp, displayWindowLabel: core.DisplayWindow}[display.Selected]
			return c.Validate()
		case 2:
			c.Workspace = strings.TrimSpace(folder.Text)
			dir, err := c.WorkspaceDir()
			if err != nil {
				return err
			}
			if err := core.RequireEmpty(dir); err != nil {
				return err
			}
			if sizeGiB, err = strconv.Atoi(strings.TrimSpace(size.Text)); err != nil || sizeGiB < core.MinDiskGiB {
				return fmt.Errorf("disk size: at least %d GiB", core.MinDiskGiB)
			}
			dl := "no: download it later from the Install tab"
			if download.Checked {
				dl = "yes, from Apple, checking its signature"
			}
			summary.SetText(fmt.Sprintf("Name: %s\nmacOS: %s (%s)\nRAM: %d MiB\nCPU: %d vCPUs (%d cores)\nDisplay: %s\nFolder: %s\nDisk: %d GiB (thin)\nDownload recovery: %s\n\nCreate makes the folder and the disk. Then start the VM and install macOS from the recovery.",
				c.Name, version.Selected, core.SMBIOSModel(c.Version), c.RAMMiB, c.VCPUs, c.Cores, display.Selected, dir, sizeGiB, dl))
		}
		return nil
	}

	cur := 0
	title := widget.NewLabel("")
	title.TextStyle = fyne.TextStyle{Bold: true}
	body := container.NewStack()
	back := widget.NewButtonWithIcon("Back", theme.NavigateBackIcon(), nil)
	next := widget.NewButtonWithIcon("Next", theme.NavigateNextIcon(), nil)
	next.Importance = widget.HighImportance
	cancel := widget.NewButton("Cancel", nil)
	show := func() {
		title.SetText(fmt.Sprintf("Step %d of %d · %s", cur+1, len(steps), steps[cur].title))
		body.Objects = []fyne.CanvasObject{steps[cur].body}
		body.Refresh()
		back.Disable()
		if cur > 0 {
			back.Enable()
		}
		next.SetText("Next")
		next.SetIcon(theme.NavigateNextIcon())
		if cur == len(steps)-1 {
			next.SetText("Create")
			next.SetIcon(theme.ConfirmIcon())
		}
	}
	d := dialog.NewCustomWithoutButtons("New VM", container.NewBorder(
		container.NewVBox(title, widget.NewSeparator()),
		container.NewVBox(widget.NewSeparator(), container.NewHBox(cancel, layout.NewSpacer(), back, next)),
		nil, nil, body), u.win)
	cancel.OnTapped = d.Hide
	back.OnTapped = func() { cur--; show() }
	next.OnTapped = func() {
		if cur < len(steps)-1 {
			if err := check(cur); err != nil {
				dialog.ShowError(err, u.win)
				return
			}
			cur++
			show()
			return
		}
		d.Hide()
		u.createVM(c, sizeGiB, download.Checked)
	}
	show()
	d.Resize(fyne.NewSize(680, 460))
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

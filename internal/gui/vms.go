package gui

import (
	"context"
	"fmt"
	"path/filepath"
	"strings"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/dialog"
	"fyne.io/fyne/v2/storage"
	"fyne.io/fyne/v2/widget"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

// loadVMs reads the VM list (or a single --config file) and selects the remembered VM.
// With no VMs yet, it creates one with the defaults, named after its macOS version.
func (u *ui) loadVMs(configPath string) error {
	if configPath != "" {
		c, err := core.LoadConfig(configPath)
		u.singleFile = true
		u.vms = []core.VMEntry{{ID: "config", Path: configPath, Config: c}}
		u.setCurrent(u.vms[0])
		return err
	}
	vms, err := core.ListVMs()
	if err == nil && len(vms) == 0 {
		c := core.Defaults()
		c.Name = "macOS Sonoma"
		c.Workspace = core.VMFolder(c.Name)
		var e core.VMEntry
		if e, err = core.CreateVM(c); err == nil {
			vms = []core.VMEntry{e}
		}
	}
	if err != nil {
		u.vms = []core.VMEntry{{ID: "default", Config: core.Defaults()}}
		u.setCurrent(u.vms[0])
		return err
	}
	u.vms = vms
	sel, _ := core.SelectedVM(vms)
	u.setCurrent(sel)
	return nil
}

func (u *ui) setCurrent(e core.VMEntry) {
	u.cur, u.cfg, u.cfgPath = e, e.Config, e.Path
}

// reloadVMs rereads the list after a change and keeps (or moves) the selection to id.
func (u *ui) reloadVMs(id string) {
	if u.singleFile {
		u.vms[0].Config = u.cfg
		u.refresh()
		return
	}
	vms, err := core.ListVMs()
	if err != nil {
		dialog.ShowError(err, u.win)
		return
	}
	u.vms = vms
	if e, ok := core.FindVM(vms, id); ok {
		u.setCurrent(e)
	} else if len(vms) > 0 {
		u.setCurrent(vms[0])
	}
	u.vmItem.Refresh()
	u.selectRow()
	u.loadSettings()
	u.refresh()
}

func (u *ui) selectRow() {
	for i, e := range u.vms {
		if e.ID == u.cur.ID {
			u.vmItem.Select(i)
		}
	}
}

// selectVM switches the app to another VM. While a VM runs, the selection stays on it.
func (u *ui) selectVM(i int) {
	if i < 0 || i >= len(u.vms) || u.vms[i].ID == u.cur.ID {
		return
	}
	if u.vm.State() != core.Stopped {
		u.selectRow()
		dialog.ShowInformation("VM running", "Shut down "+u.cur.DisplayName()+" before switching to another VM.", u.win)
		return
	}
	u.setCurrent(u.vms[i])
	if !u.singleFile {
		core.SelectVM(u.cur.ID)
	}
	u.loadSettings()
	u.refresh()
}

// deleteVMDialog removes the selected VM; deleting its files is a separate, explicit choice.
func (u *ui) deleteVMDialog() {
	if u.vm.State() != core.Stopped {
		dialog.ShowInformation("Delete VM", "Shut down the VM before deleting it.", u.win)
		return
	}
	e := u.cur
	dir, _ := e.Config.WorkspaceDir()
	purge := widget.NewCheck("Also delete its disk, install media and boot files", nil)
	files := wrapped(fmt.Sprintf("From %s: %s, %s and the %s folder. Other files in that folder are never touched.",
		dir, core.DiskFile, core.MediaFile, core.BootDir))
	files.TextStyle = fyne.TextStyle{Italic: true}
	body := container.NewVBox(
		wrapped("Remove "+e.DisplayName()+" from the list?"),
		purge,
		files,
	)
	d := dialog.NewCustomConfirm("Delete VM", "Delete", "Cancel", body, func(ok bool) {
		if !ok {
			return
		}
		do := func() {
			removed, err := core.DeleteVM(e, purge.Checked)
			for _, r := range removed {
				u.appendLog("deleted " + r)
			}
			if err != nil {
				dialog.ShowError(err, u.win)
			}
			u.reloadVMs("")
			if len(u.vms) == 0 {
				u.loadVMs("") // keep one VM in the list, as on first start
				u.reloadVMs(u.cur.ID)
			}
		}
		if !purge.Checked {
			do()
			return
		}
		dialog.ShowConfirm("Delete files", "This permanently deletes the macOS disk of "+e.DisplayName()+
			". Everything installed in that VM is lost. Continue?", func(sure bool) {
			if sure {
				do()
			}
		}, u.win)
	}, u.win)
	d.Resize(fyne.NewSize(560, 0))
	d.Show()
}

// exportVMDialog saves the selected VM as settings (.json) or as a full bundle (.tar) with
// the disk, NVRAM, SMBIOS identity and install media.
func (u *ui) exportVMDialog() {
	e := u.cur
	full := "Full VM: settings, disk, NVRAM, identity and install media (.tar)"
	kind := widget.NewRadioGroup([]string{full, "Settings only (.json)"}, nil)
	kind.SetSelected(full)
	note := wrapped("The full bundle has everything the VM needs to run on another host; it is about as big as the disk. OpenCore and OVMF are rebuilt by the app at start.")
	note.TextStyle = fyne.TextStyle{Italic: true}
	dialog.ShowCustomConfirm("Export "+e.DisplayName(), "Choose file", "Cancel", container.NewVBox(kind, note), func(ok bool) {
		if !ok {
			return
		}
		isFull := kind.Selected == full
		save := dialog.NewFileSave(func(w fyne.URIWriteCloser, err error) {
			if err != nil || w == nil {
				return
			}
			path := w.URI().Path()
			if !isFull {
				defer w.Close()
				b, err := core.ExportVM(e)
				if err == nil {
					_, err = w.Write(append(b, '\n'))
				}
				if err != nil {
					dialog.ShowError(err, u.win)
					return
				}
				dialog.ShowInformation("Export", "Settings saved to "+path, u.win)
				return
			}
			w.Close() // ExportBundle writes through its own temporary file
			bar := widget.NewProgressBar()
			prog := dialog.NewCustomWithoutButtons("Exporting "+e.DisplayName(), container.NewVBox(wrapped(path), bar), u.win)
			prog.Show()
			go func() {
				err := core.ExportBundle(context.Background(), e, path, func(done, total int64) {
					fyne.Do(func() { bar.SetValue(float64(done) / float64(max(total, 1))) })
				})
				fyne.Do(func() {
					prog.Hide()
					if err != nil {
						dialog.ShowError(err, u.win)
						return
					}
					dialog.ShowInformation("Export", e.DisplayName()+" exported to "+path, u.win)
				})
			}()
		}, u.win)
		name := core.Slug(e.DisplayName()) + map[bool]string{true: ".tar", false: ".json"}[isFull]
		save.SetFileName(name)
		save.Resize(fyne.NewSize(900, 600))
		save.Show()
	}, u.win)
}

// importVMDialog adds a VM from a settings file (.json) or a full bundle (.tar).
func (u *ui) importVMDialog() {
	open := dialog.NewFileOpen(func(r fyne.URIReadCloser, err error) {
		if err != nil || r == nil {
			return
		}
		path := r.URI().Path()
		r.Close()
		if !strings.HasSuffix(strings.ToLower(path), ".tar") {
			e, err := core.ImportSettings(path)
			if err != nil {
				dialog.ShowError(err, u.win)
				return
			}
			core.SelectVM(e.ID)
			u.reloadVMs(e.ID)
			dialog.ShowInformation("Import", e.DisplayName()+" imported. It uses the folder "+e.Config.Workspace+".", u.win)
			return
		}
		folder := widget.NewEntry()
		folder.SetText(core.VMFolder(strings.TrimSuffix(filepath.Base(path), ".tar")))
		dialog.ShowForm("Import VM bundle", "Import", "Cancel",
			[]*widget.FormItem{widget.NewFormItem("Folder", folder)}, func(ok bool) {
				if !ok {
					return
				}
				dir, err := (core.Config{Workspace: strings.TrimSpace(folder.Text)}).WorkspaceDir()
				if err != nil {
					dialog.ShowError(err, u.win)
					return
				}
				bar := widget.NewProgressBar()
				prog := dialog.NewCustomWithoutButtons("Importing", container.NewVBox(wrapped(path+" → "+dir), bar), u.win)
				prog.Show()
				go func() {
					e, err := core.ImportBundle(context.Background(), path, dir, func(done, total int64) {
						fyne.Do(func() { bar.SetValue(float64(done) / float64(max(total, 1))) })
					})
					fyne.Do(func() {
						prog.Hide()
						if err != nil {
							dialog.ShowError(err, u.win)
							return
						}
						core.SelectVM(e.ID)
						u.reloadVMs(e.ID)
						dialog.ShowInformation("Import", e.DisplayName()+" imported into "+dir+".", u.win)
					})
				}()
			}, u.win)
	}, u.win)
	open.SetFilter(storage.NewExtensionFileFilter([]string{".tar", ".json"}))
	open.Resize(fyne.NewSize(900, 600))
	open.Show()
}

// moveVMDialog moves the selected VM's files into a new folder, by default <Workspace>/<name>.
func (u *ui) moveVMDialog() {
	if u.vm.State() != core.Stopped {
		dialog.ShowInformation("Move VM", "Shut down the VM before moving it.", u.win)
		return
	}
	folder := widget.NewEntry()
	folder.SetText(core.VMFolder(u.cfg.Name))
	d := dialog.NewForm("Move VM", "Move", "Cancel", []*widget.FormItem{
		widget.NewFormItem("Current folder", widget.NewLabel(u.cfg.Workspace)),
		widget.NewFormItem("New folder", folder),
	}, func(ok bool) {
		if !ok {
			return
		}
		cur, dst := u.cur, folder.Text
		var moved core.VMEntry
		u.run("Move VM", func() (err error) {
			moved, err = core.MoveVM(cur, dst)
			return err
		}, func() {
			if moved.Path != "" { // a standalone --config file is not in the list
				u.vms = []core.VMEntry{moved}
				u.setCurrent(moved)
			}
			u.reloadVMs(moved.ID)
		})
	}, u.win)
	d.Resize(fyne.NewSize(620, 0))
	d.Show()
}

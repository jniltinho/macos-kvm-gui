// macos-kvm-gui: interface desktop para instalar e rodar macOS no KVM usando o OSX-KVM.
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

const manualSteps = `**Passos manuais dentro da VM (primeira instalação)**

1. No menu do OpenCore, escolha **macOS Base System**.
2. Abra o **Disk Utility**, ative *View › Show All Devices*, selecione o disco **QEMU HARDDISK** e use **Erase** (APFS, GUID).
3. Feche o Disk Utility e escolha **Reinstall macOS**. A VM reinicia algumas vezes: no OpenCore escolha **macOS Installer** e, depois, o disco instalado.
4. Quando o macOS já iniciar pelo disco, remova a mídia de instalação aqui para liberar espaço.`

const videoTips = `**Desempenho de vídeo**

A VM não tem aceleração gráfica: o macOS não tem driver para o vídeo virtual nem para GPUs NVIDIA, então a tela é desenhada pela CPU. Para ficar mais fluido:

1. Use **Recomendado para este host** na aba Configuração (mais vCPUs e RAM).
2. No macOS, em *System Settings › Accessibility › Display*, ative **Reduce motion** e **Reduce transparency**.
3. Em *Desktop & Dock*, escolha minimizar com **Scale** e desligue **Animate opening applications**.
4. Prefira um papel de parede estático e mantenha 1920x1080.

Aceleração de verdade só com uma GPU AMD suportada pelo macOS (por exemplo RX 580 ou RX 6600) em passthrough VFIO.`

type ui struct {
	win     fyne.Window
	cfgPath string
	cfg     core.Config
	host    core.Host
	vm      *core.VM
	busy    bool

	checks       *fyne.Container
	installInfo  *widget.Label
	stage        *widget.Label
	bar          *widget.ProgressBar
	barInf       *widget.ProgressBarInfinite
	diskSize     *widget.Entry
	vmState      *widget.Label
	log          *widget.TextGrid
	logScroll    *container.Scroll
	logLines     []string
	actionsOnOff []*widget.Button // desligados durante operações longas

	btnPrepare, btnDownload, btnDisk, btnRemove *widget.Button
	btnStart, btnPowerdown, btnKill             *widget.Button
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
		container.NewTabItemWithIcon("Configuração", theme.SettingsIcon(), u.configTab()),
		container.NewTabItemWithIcon("Instalação", theme.DownloadIcon(), u.installTab()),
		container.NewTabItemWithIcon("VM", theme.MediaPlayIcon(), u.vmTab()),
		container.NewTabItemWithIcon("Log", theme.ListIcon(), u.logTab()),
	)
	u.win.SetContent(tabs)
	u.win.Resize(fyne.NewSize(820, 600))
	u.win.SetCloseIntercept(func() {
		if u.vm.State() != core.Stopped {
			dialog.ShowInformation("VM em execução", "Desligue a VM antes de fechar o app.", u.win)
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

// workspace usa a configuração salva; cada operação recebe uma cópia.
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

// run executa fn fora da thread da UI, com os botões de ação desligados.
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
				u.appendLog("erro: " + err.Error())
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
	recheck := widget.NewButtonWithIcon("Verificar de novo", theme.ViewRefreshIcon(), u.refreshChecks)
	fix := widget.NewButtonWithIcon("Corrigir ignore_msrs", theme.SettingsIcon(), func() {
		go func() {
			err := u.host.FixIgnoreMSRs()
			fyne.Do(func() {
				switch {
				case errors.Is(err, core.ErrCancelled):
					dialog.ShowInformation("ignore_msrs", "Correção cancelada: nada foi alterado.", u.win)
				case err != nil:
					dialog.ShowError(err, u.win)
				default:
					dialog.ShowInformation("ignore_msrs", "ignore_msrs ativado e persistido em /etc/modprobe.d/macos-kvm-gui.conf.", u.win)
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
		return v.Name + " — experimental"
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
		widget.NewFormItem("Versão do macOS", version),
		widget.NewFormItem("RAM (MiB)", ram),
		widget.NewFormItem("Núcleos", cores),
		widget.NewFormItem("vCPUs (total)", vcpus),
		widget.NewFormItem("Porta SSH (127.0.0.1)", port),
	)
	form.SubmitText = "Salvar"
	form.OnSubmit = func() {
		if u.vm.State() != core.Stopped {
			dialog.ShowInformation("Configuração", "Desligue a VM antes de mudar a configuração.", u.win)
			return
		}
		c := core.Config{Workspace: strings.TrimSpace(ws.Text), Version: short[version.Selected]}
		var bad string
		for _, f := range []struct {
			name string
			e    *widget.Entry
			dst  *int
		}{{"RAM", ram, &c.RAMMiB}, {"Núcleos", cores, &c.Cores}, {"vCPUs", vcpus, &c.VCPUs}, {"Porta SSH", port, &c.SSHPort}} {
			n, err := strconv.Atoi(strings.TrimSpace(f.e.Text))
			if err != nil && bad == "" {
				bad = f.name + ": informe um número inteiro"
			}
			*f.dst = n
		}
		if bad != "" {
			dialog.ShowError(errors.New(bad), u.win)
			return
		}
		if err := core.SaveConfig(u.cfgPath, c); err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		u.cfg = c
		u.refresh()
		dialog.ShowInformation("Configuração", "Configuração salva.", u.win)
	}
	recommend := widget.NewButtonWithIcon("Recomendado para este host", theme.ComputerIcon(), func() {
		r := core.Recommended(u.cfg, runtime.NumCPU(), u.host.MemTotalMiB())
		ram.SetText(strconv.Itoa(r.RAMMiB))
		cores.SetText(strconv.Itoa(r.Cores))
		vcpus.SetText(strconv.Itoa(r.VCPUs))
	})
	note := widget.NewLabel("Sem GPU suportada o macOS desenha a tela na CPU: mais vCPUs deixam o vídeo mais fluido. " +
		"O botão acima só preenche os campos; clique em Salvar para aplicar. " +
		"CPU e placa de rede são escolhidas pela versão do macOS. Só o Sonoma foi validado de ponta a ponta.")
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

	u.btnPrepare = widget.NewButtonWithIcon("Preparar workspace (clonar OSX-KVM)", theme.FolderNewIcon(), func() {
		w, err := u.workspace()
		if err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		u.showInfinite("Clonando OSX-KVM")
		u.run("Preparar workspace", func() error { return w.Prepare(context.Background(), u.logAsync) }, nil)
	})
	u.btnDownload = widget.NewButtonWithIcon("Baixar recovery do macOS", theme.DownloadIcon(), func() {
		w, err := u.workspace()
		if err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		version := u.cfg.Version
		u.run("Baixar recovery ("+version+")", func() error {
			return w.DownloadMedia(context.Background(), version, func(p core.Progress) {
				fyne.Do(func() { u.showProgress(p) })
			}, u.logAsync)
		}, nil)
	})
	u.diskSize = widget.NewEntry()
	u.diskSize.SetText("80")
	u.btnDisk = widget.NewButtonWithIcon("Criar disco", theme.StorageIcon(), func() {
		w, err := u.workspace()
		if err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		size, err := strconv.Atoi(strings.TrimSpace(u.diskSize.Text))
		if err != nil {
			dialog.ShowError(errors.New("tamanho do disco: informe um número inteiro de GiB"), u.win)
			return
		}
		var warning string
		u.run("Criar disco", func() (err error) {
			warning, err = w.CreateDisk(context.Background(), size)
			return err
		}, func() {
			if warning != "" {
				dialog.ShowInformation("Disco criado com aviso", warning, u.win)
			}
		})
	})
	u.btnRemove = widget.NewButtonWithIcon("Remover mídia de instalação", theme.DeleteIcon(), func() {
		dialog.ShowConfirm("Remover mídia",
			"Confirme que o macOS já inicia pelo disco instalado. A BaseSystem.img será apagada.", func(ok bool) {
				if !ok {
					return
				}
				w, err := u.workspace()
				if err != nil {
					dialog.ShowError(err, u.win)
					return
				}
				var freed int64
				u.run("Remover mídia", func() (err error) {
					freed, err = w.RemoveMedia()
					return err
				}, func() {
					dialog.ShowInformation("Mídia removida", fmt.Sprintf("%.2f GiB liberados.", float64(freed)/(1<<30)), u.win)
				})
			}, u.win)
	})
	u.actionsOnOff = append(u.actionsOnOff, u.btnPrepare, u.btnDownload, u.btnDisk, u.btnRemove)

	disk := container.NewBorder(nil, nil, widget.NewLabel("Tamanho (GiB, mín. 40):"), u.btnDisk, u.diskSize)
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

func (u *ui) logAsync(s string) { fyne.Do(func() { u.appendLog(s) }) }

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
	u.stage.SetText(fmt.Sprintf("%s — %.1f%%", p.Stage, p.Pct))
	u.barInf.Stop()
	u.barInf.Hide()
	u.bar.Show()
	u.bar.SetValue(p.Pct / 100)
}

func (u *ui) vmTab() fyne.CanvasObject {
	u.vmState = widget.NewLabel("")
	u.btnStart = widget.NewButtonWithIcon("Iniciar", theme.MediaPlayIcon(), func() {
		if err := u.vm.Start(u.cfg); err != nil {
			dialog.ShowError(err, u.win)
		}
		u.refresh()
	})
	u.btnPowerdown = widget.NewButtonWithIcon("Desligar (ACPI)", theme.LogoutIcon(), func() {
		go func() {
			err := u.vm.Powerdown()
			fyne.Do(func() {
				if err != nil {
					dialog.ShowError(err, u.win)
				} else {
					u.appendLog("[VM] pedido de desligamento enviado ao macOS")
				}
			})
		}()
	})
	u.btnKill = widget.NewButtonWithIcon("Forçar parada", theme.CancelIcon(), func() {
		dialog.ShowConfirm("Forçar parada", "Equivale a puxar o cabo de energia: dados não salvos no macOS serão perdidos. Continuar?",
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

func (u *ui) logTab() fyne.CanvasObject {
	u.log = widget.NewTextGrid()
	u.logScroll = container.NewScroll(u.log)
	return u.logScroll
}

// refresh recalcula textos e botões a partir do estado real do disco e da VM.
func (u *ui) refresh() {
	if u.installInfo == nil {
		return
	}
	u.refreshChecks()
	state := u.vm.State()
	running := state != core.Stopped
	free := !u.busy && !running

	w, err := u.workspace()
	var lines []string
	st := core.WorkspaceIncomplete
	if err != nil {
		lines = append(lines, "Workspace inválido: "+err.Error())
	} else {
		var missing []string
		st, missing = w.Classify()
		switch st {
		case core.WorkspaceEmpty:
			lines = append(lines, "Workspace "+w.Dir+": vazio (clique em Preparar)")
		case core.WorkspaceReady:
			lines = append(lines, "Workspace "+w.Dir+": pronto")
		default:
			lines = append(lines, "Workspace "+w.Dir+": não é um OSX-KVM completo; faltam "+strings.Join(missing, ", "))
		}
		if w.HasMedia() {
			lines = append(lines, fmt.Sprintf("Mídia de instalação: %s (%.2f GiB)", w.MediaVersion(), float64(core.Allocated(w.Path(core.MediaFile)))/(1<<30)))
		} else {
			lines = append(lines, "Mídia de instalação: ausente")
		}
		if w.HasDisk() {
			lines = append(lines, fmt.Sprintf("Disco do macOS: %.2f GiB ocupados no host", float64(core.Allocated(w.Path(core.DiskFile)))/(1<<30)))
		} else {
			lines = append(lines, "Disco do macOS: ainda não criado")
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

	p := core.ProfileFor(u.cfg.Version)
	u.vmState.SetText(fmt.Sprintf("Estado: %s\nmacOS %s · %d MiB · %d vCPUs (%d núcleos) · CPU %s · SSH: ssh -p %d usuario@127.0.0.1",
		state, u.cfg.Version, u.cfg.RAMMiB, u.cfg.VCPUs, u.cfg.Cores, strings.SplitN(p.CPU, ",", 2)[0], u.cfg.SSHPort))
}

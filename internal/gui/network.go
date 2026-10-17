package gui

import (
	"errors"
	"fmt"
	"reflect"
	"strconv"
	"strings"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/theme"
	"fyne.io/fyne/v2/widget"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

// Settings › Network, like VirtualBox's: one tab per adapter (Adapter 1–4) with Enable,
// Attached to, the bridge name, the adapter type and the MAC address.

const (
	attachNAT    = "NAT"
	attachBridge = "Bridged Adapter"
)

// nicLabels are VirtualBox-style names for the adapter types.
var nicLabels = map[string]string{
	"virtio-net-pci": "Paravirtualized Network (virtio-net)",
	"vmxnet3":        "VMware vmxnet3",
	"e1000-82545em":  "Intel PRO/1000 MT Desktop (82545EM)",
}

func nicLabel(model string) string {
	if l, ok := nicLabels[model]; ok {
		return l
	}
	return model
}

// networkPage is the page and the function that reads it back; version gives the macOS
// version currently chosen in General, for the driver warnings. id is the VM's, so its own port
// is not reported as shared.
func networkPage(id string, c core.Config, version func() string) (fyne.CanvasObject, func() ([]core.NIC, int, error)) {
	current := c.Adapters()
	port, portRow := sshPortField(id, c.SSHPort)

	var models []string
	byLabel := map[string]string{}
	for _, m := range core.NICModels {
		models = append(models, nicLabel(m))
		byLabel[nicLabel(m)] = m
	}
	bridges := core.HostBridges()

	type tab struct {
		enabled *widget.Check
		attach  *widget.Select
		bridge  *widget.SelectEntry
		model   *widget.Select
		mac     *widget.Entry
		warn    *widget.Label
	}
	tabs := make([]tab, core.MaxNICs)
	items := make([]*container.TabItem, core.MaxNICs)
	for i := range tabs {
		n := core.NIC{Mode: core.NetNAT, Model: "", MAC: ""}
		if i < len(current) {
			n = current[i]
		}
		t := tab{
			enabled: widget.NewCheck("Enable Network Adapter", nil),
			attach:  widget.NewSelect([]string{attachNAT, attachBridge}, nil),
			bridge:  widget.NewSelectEntry(bridges),
			model:   widget.NewSelect(models, nil),
			mac:     widget.NewEntry(),
			warn:    caption(""),
		}
		t.warn.Importance = widget.WarningImportance
		t.warn.Wrapping = fyne.TextWrapWord
		t.bridge.SetPlaceHolder("br0")
		t.bridge.SetText(n.Bridge)
		t.mac.SetText(n.MAC)
		model := n.Model
		if model == "" {
			model = core.NewNetworks(version())[0].Model
		}
		t.model.SetSelected(nicLabel(model))
		t.attach.SetSelected(map[bool]string{true: attachBridge, false: attachNAT}[n.Mode == core.NetBridged])

		update := func() {
			on := t.enabled.Checked
			for _, w := range []fyne.Disableable{t.attach, t.model, t.mac} {
				if on {
					w.Enable()
				} else {
					w.Disable()
				}
			}
			if on && t.attach.Selected == attachBridge {
				t.bridge.Enable()
			} else {
				t.bridge.Disable()
			}
			if on && strings.TrimSpace(t.mac.Text) == "" {
				t.mac.SetText(core.RandomMAC()) // a new adapter gets its own address
			}
			var msg []string
			if on && !core.NICSupported(version(), byLabel[t.model.Selected]) {
				msg = append(msg, versionTitle(version())+" has no built-in driver for this adapter type; choose another unless the guest has one.")
			}
			if on && t.attach.Selected == attachBridge {
				if core.BridgeHelper() == "" {
					msg = append(msg, "qemu-bridge-helper is not installed.")
				}
				if len(bridges) == 0 {
					msg = append(msg, "The host has no Linux bridge yet: create one (for example br0) with your network tools.")
				}
				msg = append(msg, "Bridged adapters need qemu-bridge-helper to be setuid root and the bridge allowed in /etc/qemu/bridge.conf; see Troubleshooting › Bridged networking.")
			}
			t.warn.SetText(strings.Join(msg, " "))
			t.warn.Hidden = len(msg) == 0
		}
		t.enabled.OnChanged = func(bool) { update() }
		t.attach.OnChanged = func(string) { update() }
		t.model.OnChanged = func(string) { update() }
		t.enabled.SetChecked(i < len(current) && n.Enabled)
		update()

		regen := widget.NewButtonWithIcon("", theme.ViewRefreshIcon(), func() { t.mac.SetText(core.RandomMAC()) })
		form := widget.NewForm(
			widget.NewFormItem("Attached to", t.attach),
			widget.NewFormItem("Name", t.bridge),
			widget.NewFormItem("Adapter Type", t.model),
			widget.NewFormItem("MAC Address", container.NewBorder(nil, nil, nil, regen, t.mac)),
		)
		tabs[i] = t
		items[i] = container.NewTabItem(fmt.Sprintf("Adapter %d", i+1), container.NewVBox(t.enabled, form, t.warn))
	}

	read := func() ([]core.NIC, int, error) {
		var nics []core.NIC
		last := -1
		for i, t := range tabs {
			n := core.NIC{
				Enabled: t.enabled.Checked,
				Mode:    map[string]string{attachNAT: core.NetNAT, attachBridge: core.NetBridged}[t.attach.Selected],
				Model:   byLabel[t.model.Selected],
				MAC:     strings.ToLower(strings.TrimSpace(t.mac.Text)),
			}
			if n.Mode == core.NetBridged {
				n.Bridge = strings.TrimSpace(t.bridge.Text)
			}
			if n.Enabled || i < len(current) {
				last = i
			}
			if n.MAC == "" { // an adapter that was never enabled
				n.MAC = core.RandomMAC()
			}
			nics = append(nics, n)
		}
		nics = nics[:last+1] // trailing never-used tabs are not stored
		p, err := strconv.Atoi(strings.TrimSpace(port.Text))
		if err != nil {
			return nil, 0, errors.New("SSH port: enter a whole number")
		}
		if c.Networks == nil && reflect.DeepEqual(nics, current) {
			nics = nil // an untouched legacy VM keeps its implicit adapter
		}
		return nics, p, nil
	}

	note := caption("The SSH port forward (127.0.0.1 → guest port 22) goes on the first enabled NAT adapter. " +
		"Changing the first adapter's MAC or type can make macOS see a new network interface; " +
		"iCloud and Messages may then ask you to sign in again.")
	note.Wrapping = fyne.TextWrapWord
	page := container.NewVBox(container.NewAppTabs(items...),
		widget.NewForm(widget.NewFormItem("SSH port on 127.0.0.1", portRow)), note)
	return page, read
}

// sshPortField is the SSH port entry with a "Use a free port" button and, under it, a warning
// when another VM uses the port (saving is still allowed: the two cannot run at the same time).
// id is the VM's own id ("" for a new VM).
func sshPortField(id string, value int) (*widget.Entry, fyne.CanvasObject) {
	port := widget.NewEntry()
	warn := caption("")
	warn.Importance = widget.WarningImportance
	warn.Wrapping = fyne.TextWrapWord
	port.OnChanged = func(t string) {
		msg := ""
		if p, err := strconv.Atoi(strings.TrimSpace(t)); err == nil {
			msg = core.SSHPortWarning(p, id)
		}
		warn.SetText(msg)
		warn.Hidden = msg == ""
	}
	port.SetText(strconv.Itoa(value))
	free := widget.NewButton("Use a free port", func() {
		p, err := core.NextSSHPort(id, nil)
		if err != nil {
			warn.SetText(err.Error())
			warn.Show()
			return
		}
		port.SetText(strconv.Itoa(p))
	})
	return port, container.NewVBox(container.NewBorder(nil, nil, nil, free, port), warn)
}

// adapterSummary is a Details line for an adapter, like VirtualBox's "Intel PRO/1000 MT Desktop (NAT)".
func adapterSummary(n core.NIC) string {
	if n.Mode == core.NetBridged {
		return fmt.Sprintf("%s (Bridged Adapter, %s)", nicLabel(n.Model), n.Bridge)
	}
	return nicLabel(n.Model) + " (NAT)"
}

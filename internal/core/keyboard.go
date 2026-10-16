package core

import (
	"bufio"
	"os"
	"os/exec"
	"strconv"
	"strings"
)

// The recovery and the installer use the keyboard layout in NVRAM prev-lang:kbd. Keys reach
// macOS as the physical keys they are (QEMU extended key events, without -k), so that layout
// should be the host's: a Brazilian ABNT2 keyboard then types "/", ":" and "ç" where its
// keycaps show them. The interface language stays English.

// macKeyboardIDs maps XKB layouts (and layout(variant)) to macOS keyboard layout IDs, from
// OpenCore's dump of Apple's AppleKeyboardLayouts bundle. Unknown layouts fall back to U.S.
var macKeyboardIDs = map[string]int{
	"us": 0, "us(dvorak)": 16300, "gb": 250, "de": 3, "fr": 60, "es": 87, "it": 223, "pt": 10,
	"br": 128, "br(abnt2)": 128, "latam": 89, "ch": 19, "ch(fr)": 18, "ca(fr)": -19336, "ca": -19336,
	"ru": 19458, "tr": -36, "se": 224, "no": 12, "dk": 9, "fi": 17, "pl": 30788,
}

// RecoveryKeyboard is the prev-lang:kbd value: English with the host's keyboard layout.
func RecoveryKeyboard() string {
	layout, variant := hostKeyboard()
	return "en-US:" + strconv.Itoa(macKeyboardID(layout, variant))
}

func macKeyboardID(layout, variant string) int {
	if variant != "" {
		if id, ok := macKeyboardIDs[layout+"("+variant+")"]; ok {
			return id
		}
	}
	return macKeyboardIDs[layout] // 0 (U.S.) when unknown
}

// hostKeyboard reads the system keyboard layout: /etc/default/keyboard (Debian, Ubuntu), then
// localectl (Fedora and others). Only the first of several layouts counts.
func hostKeyboard() (layout, variant string) {
	if f, err := os.Open("/etc/default/keyboard"); err == nil {
		defer f.Close()
		sc := bufio.NewScanner(f)
		for sc.Scan() {
			k, v, ok := strings.Cut(strings.TrimSpace(sc.Text()), "=")
			if !ok {
				continue
			}
			v = strings.Trim(v, `"'`)
			switch k {
			case "XKBLAYOUT":
				layout = v
			case "XKBVARIANT":
				variant = v
			}
		}
	}
	if layout == "" {
		if out, err := exec.Command("localectl", "status").Output(); err == nil {
			for _, line := range strings.Split(string(out), "\n") {
				k, v, ok := strings.Cut(strings.TrimSpace(line), ":")
				if !ok {
					continue
				}
				switch strings.TrimSpace(k) {
				case "X11 Layout":
					layout = strings.TrimSpace(v)
				case "X11 Variant":
					variant = strings.TrimSpace(v)
				}
			}
		}
	}
	layout, _, _ = strings.Cut(layout, ",")
	variant, _, _ = strings.Cut(variant, ",")
	return strings.TrimSpace(layout), strings.TrimSpace(variant)
}

// GuestTypingOK reports whether the app can type into the guest: GuestKeys's keys give their
// characters in the guest's layout (the host's), except where letters move, as on Dvorak, or
// are not Latin, as on Russian.
func GuestTypingOK() bool {
	id := macKeyboardID(hostKeyboard())
	return id != macKeyboardIDs["us(dvorak)"] && id != macKeyboardIDs["ru"]
}

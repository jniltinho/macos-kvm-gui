package gui

import (
	"image/color"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/theme"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

// appTheme is Fyne's default theme with a macOS-like accent, roomier spacing and square corners.
// mode is "dark" (default), "light" or "system" (follow the desktop).
type appTheme struct {
	fyne.Theme
	mode string
}

// themeMode picks the theme: MKG_THEME (environment or .env) wins, then the saved preference.
func themeMode() string {
	for _, m := range []string{core.Setting("MKG_THEME"), core.Preference("theme")} {
		if m == "dark" || m == "light" || m == "system" {
			return m
		}
	}
	return "dark"
}

var accent = color.NRGBA{R: 0x0a, G: 0x84, B: 0xff, A: 0xff}

// colorListBackground is the VM list's own background, a shade lighter than the window in the
// light theme like VirtualBox's.
const colorListBackground fyne.ThemeColorName = "listBackground"

func rgb(v uint32) color.NRGBA {
	return color.NRGBA{R: uint8(v >> 16), G: uint8(v >> 8), B: uint8(v), A: 0xff}
}

// light is the light palette, measured from VirtualBox's manager on KDE (Breeze colors).
var light = map[fyne.ThemeColorName]color.Color{
	theme.ColorNameBackground:          rgb(0xEBEFF4), // window, toolbar, menus, dialogs
	colorListBackground:                rgb(0xF9FAFB),
	theme.ColorNameHeaderBackground:    rgb(0xDCE0E5), // section bands
	theme.ColorNameForeground:          rgb(0x232629),
	theme.ColorNameInputBackground:     rgb(0xFFFFFF),
	theme.ColorNameInputBorder:         rgb(0xBCC0C4),
	theme.ColorNameButton:              rgb(0xF6FAFC),
	theme.ColorNameDisabledButton:      rgb(0xE3E6EA),
	theme.ColorNameDisabled:            rgb(0xA0A4A8),
	theme.ColorNamePlaceHolder:         rgb(0x8C9094),
	theme.ColorNameSeparator:           rgb(0xC2C5C9),
	theme.ColorNameMenuBackground:      rgb(0xFCFCFC),
	theme.ColorNameOverlayBackground:   rgb(0xFCFCFC),
	theme.ColorNameSelection:           rgb(0x41B6F4), // selected row, solid like VirtualBox's
	theme.ColorNameHover:               color.NRGBA{R: 0x3D, G: 0xAE, B: 0xE9, A: 0x33},
	theme.ColorNameFocus:               rgb(0x3DAEE9),
	theme.ColorNamePressed:             color.NRGBA{R: 0x3D, G: 0xAE, B: 0xE9, A: 0x66},
	theme.ColorNameScrollBar:           color.NRGBA{R: 0x23, G: 0x26, B: 0x29, A: 0x66},
	theme.ColorNameScrollBarBackground: color.NRGBA{R: 0x23, G: 0x26, B: 0x29, A: 0x14},
	theme.ColorNameShadow:              color.NRGBA{A: 0x33},
	theme.ColorNameSuccess:             rgb(0x27AE60),
	theme.ColorNameWarning:             rgb(0xF67400),
	theme.ColorNameError:               rgb(0xDA4453),
}

func (t appTheme) Color(n fyne.ThemeColorName, v fyne.ThemeVariant) color.Color {
	switch t.mode {
	case "light":
		v = theme.VariantLight
	case "system":
	default:
		v = theme.VariantDark
	}
	if v == theme.VariantLight {
		if c, ok := light[n]; ok {
			return c
		}
	} else if n == colorListBackground {
		n = theme.ColorNameBackground
	}
	switch n {
	case theme.ColorNamePrimary, theme.ColorNameHyperlink:
		return accent
	case theme.ColorNameFocus:
		return color.NRGBA{R: 0x0a, G: 0x84, B: 0xff, A: 0x66}
	case theme.ColorNameSelection:
		return color.NRGBA{R: 0x0a, G: 0x84, B: 0xff, A: 0x33}
	}
	return t.Theme.Color(n, v)
}

func (t appTheme) Size(n fyne.ThemeSizeName) float32 {
	switch n {
	case theme.SizeNameInputRadius, theme.SizeNameSelectionRadius, theme.SizeNameScrollBarRadius,
		theme.SizeNameButtonRadius, theme.SizeNameInnerWindowRadius, theme.SizeNameWindowButtonRadius,
		theme.SizeNameDialogRadius, theme.SizeNamePopupRadius, theme.SizeNameCardRadius:
		return 0 // square corners everywhere, dialogs, alerts, menus and cards included
	case theme.SizeNamePadding:
		return 4
	case theme.SizeNameInnerPadding:
		return 7
	case theme.SizeNameText:
		return 13
	}
	return t.Theme.Size(n)
}

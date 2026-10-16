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

func (t appTheme) Color(n fyne.ThemeColorName, v fyne.ThemeVariant) color.Color {
	switch t.mode {
	case "light":
		v = theme.VariantLight
	case "system":
	default:
		v = theme.VariantDark
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
		theme.SizeNameButtonRadius, theme.SizeNameInnerWindowRadius:
		return 0
	case theme.SizeNamePadding:
		return 6
	case theme.SizeNameInnerPadding:
		return 10
	}
	return t.Theme.Size(n)
}

package main

import (
	"image/color"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/theme"
)

// appTheme is Fyne's default theme with a macOS-like accent, roomier spacing and square corners.
// It is dark unless light is true (MKG_THEME=light in the environment or the local .env).
type appTheme struct {
	fyne.Theme
	light bool
}

var accent = color.NRGBA{R: 0x0a, G: 0x84, B: 0xff, A: 0xff}

func (t appTheme) Color(n fyne.ThemeColorName, v fyne.ThemeVariant) color.Color {
	v = theme.VariantDark
	if t.light {
		v = theme.VariantLight
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
	case theme.SizeNameInputRadius, theme.SizeNameSelectionRadius, theme.SizeNameScrollBarRadius:
		return 0
	case theme.SizeNamePadding:
		return 6
	case theme.SizeNameInnerPadding:
		return 10
	}
	return t.Theme.Size(n)
}

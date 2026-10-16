package gui

import (
	"fmt"
	"image/color"
	"strconv"
	"strings"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/canvas"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/driver/desktop"
	"fyne.io/fyne/v2/layout"
	"fyne.io/fyne/v2/theme"
	"fyne.io/fyne/v2/widget"
)

// VirtualBox-style building blocks: toolbar buttons with a large icon over the label, compact
// key/value text, section bands and slider rows.

// themed holds the recolor functions of plain canvas objects, which Fyne does not restyle on a
// theme change by itself; Run calls them from the settings listener.
var themed []func()

func recolorThemed() {
	for _, f := range themed {
		f()
	}
}

// themedRect is a rectangle filled with a theme color that follows theme changes.
func themedRect(name fyne.ThemeColorName) *canvas.Rectangle {
	r := canvas.NewRectangle(theme.Color(name))
	themed = append(themed, func() { r.FillColor = theme.Color(name); r.Refresh() })
	return r
}

// themedText is compact text (no label padding) in the foreground color.
func themedText(s string, bold bool) *canvas.Text {
	t := canvas.NewText(s, theme.Color(theme.ColorNameForeground))
	t.TextStyle = fyne.TextStyle{Bold: bold}
	themed = append(themed, func() { t.Color = theme.Color(theme.ColorNameForeground); t.Refresh() })
	return t
}

// powerIcon is VirtualBox's state symbol for a powered-off VM (square caps, no rounded corners).
var powerIcon = theme.NewThemedResource(fyne.NewStaticResource("power.svg", []byte(
	`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M11 2h2v10h-2z"/>`+
		`<path d="M7.05 5.46l1.41 1.42A7 7 0 1 0 15.54 6.88l1.41-1.42A9 9 0 1 1 7.05 5.46z"/></svg>`)))

// toolButton is a flat toolbar button: a 32 px colored icon over its label, like VirtualBox.
type toolButton struct {
	widget.DisableableWidget
	label   string
	icon    fyne.Resource
	color   fyne.ThemeColorName
	OnTap   func()
	hovered bool

	bg   *canvas.Rectangle
	img  *canvas.Image
	text *canvas.Text
}

func newToolButton(label string, icon fyne.Resource, c fyne.ThemeColorName, tap func()) *toolButton {
	b := &toolButton{label: label, icon: icon, color: c, OnTap: tap}
	b.ExtendBaseWidget(b)
	return b
}

func (b *toolButton) resource() fyne.Resource {
	if b.Disabled() {
		return theme.NewDisabledResource(b.icon)
	}
	return tint(b.icon, b.color)
}

// tint colors a monochrome theme icon; the app's own multicolor icons are returned unchanged.
func tint(r fyne.Resource, c fyne.ThemeColorName) fyne.Resource {
	if _, own := r.(*fyne.StaticResource); own {
		return r
	}
	return theme.NewColoredResource(r, c)
}

func (b *toolButton) CreateRenderer() fyne.WidgetRenderer {
	b.bg = canvas.NewRectangle(color.Transparent)
	b.img = canvas.NewImageFromResource(b.resource())
	b.img.FillMode = canvas.ImageFillContain
	b.img.SetMinSize(fyne.NewSquareSize(32))
	b.text = canvas.NewText(b.label, theme.Color(theme.ColorNameForeground))
	b.text.Alignment = fyne.TextAlignCenter
	b.update()
	body := container.New(layout.NewCustomPaddedVBoxLayout(2), container.NewCenter(b.img), b.text)
	return widget.NewSimpleRenderer(container.NewStack(b.bg, container.NewPadded(body)))
}

func (b *toolButton) Refresh() {
	b.update()
	b.DisableableWidget.Refresh()
}

// update restyles the parts; CreateRenderer uses it too, since Refresh asks for the renderer.
func (b *toolButton) update() {
	if b.img != nil {
		b.img.Resource = b.resource()
		b.img.Refresh()
		b.text.Color = theme.Color(theme.ColorNameForeground)
		if b.Disabled() {
			b.text.Color = theme.Color(theme.ColorNameDisabled)
		}
		b.text.Refresh()
		b.bg.FillColor = color.Transparent
		if b.hovered && !b.Disabled() {
			b.bg.FillColor = theme.Color(theme.ColorNameHover)
		}
		b.bg.Refresh()
	}
}

func (b *toolButton) Tapped(*fyne.PointEvent) {
	if !b.Disabled() && b.OnTap != nil {
		b.OnTap()
	}
}

func (b *toolButton) MouseIn(*desktop.MouseEvent)    { b.hovered = true; b.Refresh() }
func (b *toolButton) MouseMoved(*desktop.MouseEvent) {}
func (b *toolButton) MouseOut()                      { b.hovered = false; b.Refresh() }

// trigger runs the button's action from a menu, only when the button itself is enabled.
func (b *toolButton) trigger() {
	if !b.Disabled() && b.OnTap != nil {
		b.OnTap()
	}
}

// infoSection is a Details section: a band with an icon and a bold title over compact
// "Key:  value" lines. It returns the value texts by key.
func infoSection(title string, icon fyne.Resource, keys ...string) (fyne.CanvasObject, map[string]*canvas.Text) {
	values := map[string]*canvas.Text{}
	grid := container.New(kvLayout{})
	for _, k := range keys {
		v := themedText("", false)
		values[k] = v
		grid.Add(themedText(k+":", false))
		grid.Add(v)
	}
	return container.NewVBox(sectionBand(title, icon), container.NewPadded(grid)), values
}

// sectionBand is the tinted title row of a Details or Settings section.
func sectionBand(title string, icon fyne.Resource) fyne.CanvasObject {
	head := container.NewHBox(widget.NewIcon(tint(icon, theme.ColorNamePrimary)), themedText(title, true))
	return container.NewStack(themedRect(theme.ColorNameHeaderBackground), container.NewPadded(head))
}

// kvLayout lays out key/value pairs as compact rows (VirtualBox's details lines): keys in one
// column as wide as the widest key, values after it, a small gap between rows.
type kvLayout struct{}

const kvRowGap, kvColGap = 1, 14

// kvValueWidth caps how much width a value asks for, so long values do not force the whole
// pane wide (the VM list could not grow); longer text is clipped by the scroll view.
const kvValueWidth = 240

func (kvLayout) keyWidth(objs []fyne.CanvasObject) float32 {
	var w float32
	for i := 0; i < len(objs); i += 2 {
		w = max(w, objs[i].MinSize().Width)
	}
	return w
}

func (l kvLayout) MinSize(objs []fyne.CanvasObject) fyne.Size {
	var w, h float32
	for i := 0; i+1 < len(objs); i += 2 {
		k, v := objs[i].MinSize(), objs[i+1].MinSize()
		w = max(w, min(v.Width, kvValueWidth))
		h += max(k.Height, v.Height) + kvRowGap
	}
	return fyne.NewSize(l.keyWidth(objs)+kvColGap+w, h)
}

func (l kvLayout) Layout(objs []fyne.CanvasObject, size fyne.Size) {
	kw := l.keyWidth(objs)
	var y float32
	for i := 0; i+1 < len(objs); i += 2 {
		k, v := objs[i], objs[i+1]
		rh := max(k.MinSize().Height, v.MinSize().Height)
		k.Move(fyne.NewPos(0, y))
		k.Resize(fyne.NewSize(kw, rh))
		v.Move(fyne.NewPos(kw+kvColGap, y))
		v.Resize(fyne.NewSize(max(size.Width-kw-kvColGap, 0), rh))
		y += rh + kvRowGap
	}
}

// sliderRow is VirtualBox's slider with a value box and the range below it. value reads the
// box; set moves both.
func sliderRow(minV, maxV, step float64, unit string) (row fyne.CanvasObject, value func() (int, error), set func(int)) {
	s := widget.NewSlider(minV, maxV)
	s.Step = step
	e := widget.NewEntry()
	syncing := false
	s.OnChanged = func(v float64) {
		if !syncing {
			syncing = true
			e.SetText(strconv.Itoa(int(v)))
			syncing = false
		}
	}
	e.OnChanged = func(t string) {
		if v, err := strconv.Atoi(strings.TrimSpace(t)); err == nil && !syncing {
			syncing = true
			s.SetValue(min(max(float64(v), minV), maxV))
			syncing = false
		}
	}
	set = func(v int) { e.SetText(strconv.Itoa(v)); s.SetValue(float64(v)) }
	value = func() (int, error) {
		v, err := strconv.Atoi(strings.TrimSpace(e.Text))
		if err != nil {
			return 0, fmt.Errorf("enter a whole number of %s", unit)
		}
		if float64(v) < minV || float64(v) > maxV {
			return 0, fmt.Errorf("choose between %s and %s", amount(int(minV), unit), amount(int(maxV), unit))
		}
		return v, nil
	}
	box := container.NewBorder(nil, nil, nil, widget.NewLabel(unit), e)
	scale := container.NewHBox(caption(amount(int(minV), unit)), layout.NewSpacer(), caption(amount(int(maxV), unit)))
	sizer := canvas.NewRectangle(color.Transparent)
	sizer.SetMinSize(fyne.NewSize(130, 0))
	// The value box sits beside the slider, the range below the slider only.
	row = container.NewBorder(nil, nil, nil, container.NewVBox(container.NewStack(sizer, box)), container.NewVBox(s, scale))
	return row, value, set
}

// amount is "1 CPU" or "24 CPUs"; units such as MB do not change.
func amount(n int, unit string) string {
	if n == 1 && strings.HasSuffix(unit, "s") {
		unit = strings.TrimSuffix(unit, "s")
	}
	return fmt.Sprintf("%d %s", n, unit)
}

func caption(s string) *widget.Label {
	l := widget.NewLabel(s)
	l.SizeName = theme.SizeNameCaptionText
	return l
}

package core

// KeyCode is what the VM receives for one physical key: an X11 keysym (US layout) and the
// XT set 1 scancode, where 0xE0-prefixed keys have bit 7 set, as QEMU's extended key events expect.
type KeyCode struct {
	Keysym   uint32
	Scancode uint32
}

// keyCodes maps Fyne key names (fyne.KeyName and desktop key names) to key codes.
var keyCodes = map[string]KeyCode{
	"Escape": {0xff1b, 0x01}, "Return": {0xff0d, 0x1c}, "Tab": {0xff09, 0x0f}, "BackSpace": {0xff08, 0x0e},
	"Insert": {0xff63, 0xd2}, "Delete": {0xffff, 0xd3}, "Home": {0xff50, 0xc7}, "End": {0xff57, 0xcf},
	"Prior": {0xff55, 0xc9}, "Next": {0xff56, 0xd1},
	"Left": {0xff51, 0xcb}, "Up": {0xff52, 0xc8}, "Right": {0xff53, 0xcd}, "Down": {0xff54, 0xd0},
	"F1": {0xffbe, 0x3b}, "F2": {0xffbf, 0x3c}, "F3": {0xffc0, 0x3d}, "F4": {0xffc1, 0x3e},
	"F5": {0xffc2, 0x3f}, "F6": {0xffc3, 0x40}, "F7": {0xffc4, 0x41}, "F8": {0xffc5, 0x42},
	"F9": {0xffc6, 0x43}, "F10": {0xffc7, 0x44}, "F11": {0xffc8, 0x57}, "F12": {0xffc9, 0x58},
	"KP_Enter": {0xff8d, 0x9c}, "Space": {0x20, 0x39},
	"-": {'-', 0x0c}, "=": {'=', 0x0d}, "[": {'[', 0x1a}, "]": {']', 0x1b}, "\\": {'\\', 0x2b},
	";": {';', 0x27}, "'": {'\'', 0x28}, ",": {',', 0x33}, ".": {'.', 0x34}, "/": {'/', 0x35}, "`": {'`', 0x29},
	"LeftShift": {0xffe1, 0x2a}, "RightShift": {0xffe2, 0x36},
	"LeftControl": {0xffe3, 0x1d}, "RightControl": {0xffe4, 0x9d},
	"LeftAlt": {0xffe9, 0x38}, "RightAlt": {0xffea, 0xb8}, // Option in macOS
	"LeftSuper": {0xffeb, 0xdb}, "RightSuper": {0xffec, 0xdc}, // Command in macOS
	"Menu": {0xff67, 0xdd}, "CapsLock": {0xffe5, 0x3a},
}

func init() {
	for i, sc := range []uint32{0x0b, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a} { // 0-9
		keyCodes[string(rune('0'+i))] = KeyCode{uint32('0' + i), sc}
	}
	letters := map[rune]uint32{
		'Q': 0x10, 'W': 0x11, 'E': 0x12, 'R': 0x13, 'T': 0x14, 'Y': 0x15, 'U': 0x16, 'I': 0x17, 'O': 0x18, 'P': 0x19,
		'A': 0x1e, 'S': 0x1f, 'D': 0x20, 'F': 0x21, 'G': 0x22, 'H': 0x23, 'J': 0x24, 'K': 0x25, 'L': 0x26,
		'Z': 0x2c, 'X': 0x2d, 'C': 0x2e, 'V': 0x2f, 'B': 0x30, 'N': 0x31, 'M': 0x32,
	}
	for l, sc := range letters {
		keyCodes[string(l)] = KeyCode{uint32(l - 'A' + 'a'), sc} // lowercase keysym: Shift travels as its own key
	}
}

// KeyFor returns the key code for a Fyne key name.
func KeyFor(name string) (KeyCode, bool) {
	k, ok := keyCodes[name]
	return k, ok
}

// FitRect computes where a w×h frame is drawn inside a box of size bw×bh when scaled to fit
// without distortion: the scale and the offset of its top-left corner.
func FitRect(w, h int, bw, bh float32) (scale, ox, oy float32) {
	if w <= 0 || h <= 0 || bw <= 0 || bh <= 0 {
		return 0, 0, 0
	}
	scale = min(bw/float32(w), bh/float32(h))
	return scale, (bw - float32(w)*scale) / 2, (bh - float32(h)*scale) / 2
}

// ToFrame converts a position inside the box into frame coordinates. ok is false in the
// letterbox bars outside the frame.
func ToFrame(w, h int, bw, bh, px, py float32) (x, y int, ok bool) {
	scale, ox, oy := FitRect(w, h, bw, bh)
	if scale == 0 {
		return 0, 0, false
	}
	fx, fy := (px-ox)/scale, (py-oy)/scale
	if fx < 0 || fy < 0 || fx >= float32(w) || fy >= float32(h) {
		return 0, 0, false
	}
	return int(fx), int(fy), true
}

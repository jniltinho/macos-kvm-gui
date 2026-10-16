package core

import (
	"errors"
	"fmt"
	"strings"
)

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

// KeyForX11 returns the key code for a physical key given as an X11 keycode (what GLFW and Fyne
// report on Linux: the evdev code + 8). It is layout independent: every key of the keyboard,
// including the extra keys of ISO and ABNT2 keyboards (the key left of Z, ABNT2's "/?" and
// keypad "." keys), reaches macOS as the key it physically is, and macOS's own input source
// decides the character. The keysym is the US one when known; QEMU uses the scancode.
func KeyForX11(keycode int) (KeyCode, bool) {
	evdev := keycode - 8
	if evdev <= 0 {
		return KeyCode{}, false
	}
	sc, ok := evdevToXT[evdev]
	if !ok && evdev <= 88 && evdev != 84 {
		sc, ok = uint32(evdev), true // codes 1-88 equal XT set 1 (KEY_ESC=1 ... KEY_F12=88)
	}
	if !ok {
		return KeyCode{}, false
	}
	k := KeyCode{Scancode: sc}
	for _, c := range keyCodes {
		if c.Scancode == sc {
			k.Keysym = c.Keysym
			break
		}
	}
	return k, true
}

// evdevToXT holds the evdev codes that are not simply equal to their XT set 1 scancode
// (Linux input-event-codes.h; 0xE0-prefixed keys have bit 7 set, as elsewhere in this file).
var evdevToXT = map[int]uint32{
	85:  0x76, // KEY_ZENKAKUHANKAKU
	86:  0x56, // KEY_102ND: the key left of Z on ISO keyboards ("\|" on ABNT2)
	89:  0x73, // KEY_RO: ABNT2 "/?" key (and JIS ro)
	92:  0x79, // KEY_HENKAN
	93:  0x70, // KEY_KATAKANAHIRAGANA
	94:  0x7b, // KEY_MUHENKAN
	96:  0x9c, // KEY_KPENTER
	97:  0x9d, // KEY_RIGHTCTRL
	98:  0xb5, // KEY_KPSLASH
	99:  0x54, // KEY_SYSRQ (QEMU's qnum for Print/SysRq)
	100: 0xb8, // KEY_RIGHTALT
	102: 0xc7, // KEY_HOME
	103: 0xc8, // KEY_UP
	104: 0xc9, // KEY_PAGEUP
	105: 0xcb, // KEY_LEFT
	106: 0xcd, // KEY_RIGHT
	107: 0xcf, // KEY_END
	108: 0xd0, // KEY_DOWN
	109: 0xd1, // KEY_PAGEDOWN
	110: 0xd2, // KEY_INSERT
	111: 0xd3, // KEY_DELETE
	117: 0x59, // KEY_KPEQUAL
	119: 0xc6, // KEY_PAUSE (QEMU's qnum)
	121: 0x7e, // KEY_KPCOMMA: ABNT2 keypad "."
	124: 0x7d, // KEY_YEN
	125: 0xdb, // KEY_LEFTMETA: Command
	126: 0xdc, // KEY_RIGHTMETA: Command
	127: 0xdd, // KEY_COMPOSE: Menu
}

// GuestKeys turns a line the app types into the guest into key chords, then Return. Only keys
// that give the same character in every Latin layout macOS may use are allowed: letters other
// than A, M, Q, W, Y and Z (AZERTY and QWERTZ move those), space, and the keypad's "/", "*"
// and "-". Upper case adds Shift.
func GuestKeys(line string) ([][]KeyCode, error) {
	shift := keyCodes["LeftShift"]
	keypad := map[rune]KeyCode{'/': {0xffaf, 0xb5}, '*': {0xffaa, 0x37}, '-': {0xffad, 0x4a}, ' ': keyCodes["Space"]}
	var out [][]KeyCode
	for _, r := range line {
		if k, ok := keypad[r]; ok {
			out = append(out, []KeyCode{k})
			continue
		}
		up := r >= 'A' && r <= 'Z'
		if !up && r >= 'a' && r <= 'z' {
			r -= 'a' - 'A'
		}
		k, ok := keyCodes[string(r)]
		if !ok || r < 'A' || r > 'Z' || strings.ContainsRune("AMQWYZ", r) {
			return nil, fmt.Errorf("cannot type %q in every keyboard layout", r)
		}
		if up {
			out = append(out, []KeyCode{shift, k})
		} else {
			out = append(out, []KeyCode{k})
		}
	}
	return append(out, []KeyCode{keyCodes["Return"]}), nil
}

// RecoveryTerminal opens Utilities › Terminal from the recovery's window through the menu bar:
// Control+F2 focuses it (Apple, Recovery, File, Edit, Utilities), Down opens Utilities, "t"
// picks Terminal. Big Sur's recovery has no Shift+Command+T shortcut; this works from Big Sur
// to Tahoe. (A func: the letters join keyCodes in init, after package variables are set.)
func RecoveryTerminal() [][]KeyCode {
	right, down := []KeyCode{keyCodes["Right"]}, []KeyCode{keyCodes["Down"]}
	return [][]KeyCode{{keyCodes["LeftControl"], keyCodes["F2"]}, right, right, right, right, down,
		{keyCodes["T"]}, {keyCodes["Return"]}}
}

// keyAliases are the names ParseChord accepts besides keyCodes' own (letters, digits, F1–F12,
// "Return" and so on), case-insensitively.
var keyAliases = map[string]string{
	"shift": "LeftShift", "ctrl": "LeftControl", "control": "LeftControl", "alt": "LeftAlt", "option": "LeftAlt",
	"cmd": "LeftSuper", "command": "LeftSuper", "super": "LeftSuper", "meta": "LeftSuper",
	"enter": "Return", "return": "Return", "esc": "Escape", "escape": "Escape", "tab": "Tab", "space": "Space",
	"backspace": "BackSpace", "delete": "Delete", "home": "Home", "end": "End", "pageup": "Prior", "pagedown": "Next",
	"left": "Left", "right": "Right", "up": "Up", "down": "Down", "capslock": "CapsLock", "kp_enter": "KP_Enter",
}

var keypadCodes = map[string]KeyCode{
	"kp_divide": {0xffaf, 0xb5}, "kp_multiply": {0xffaa, 0x37}, "kp_subtract": {0xffad, 0x4a}, "kp_add": {0xffab, 0x4e},
	"kp_decimal": {0xffae, 0x53},
	"kp_0":       {0xffb0, 0x52}, "kp_1": {0xffb1, 0x4f}, "kp_2": {0xffb2, 0x50}, "kp_3": {0xffb3, 0x51}, "kp_4": {0xffb4, 0x4b},
	"kp_5": {0xffb5, 0x4c}, "kp_6": {0xffb6, 0x4d}, "kp_7": {0xffb7, 0x47}, "kp_8": {0xffb8, 0x48}, "kp_9": {0xffb9, 0x49},
}

// ParseChord turns "shift+cmd+t" into keys, pressed in that order. Names are physical keys on
// a U.S. keyboard (the guest's layout decides the character).
func ParseChord(s string) ([]KeyCode, error) {
	var out []KeyCode
	for _, part := range strings.Split(s, "+") {
		name := strings.TrimSpace(part)
		low := strings.ToLower(name)
		if k, ok := keypadCodes[low]; ok {
			out = append(out, k)
			continue
		}
		if a, ok := keyAliases[low]; ok {
			name = a
		} else if len(name) == 1 {
			name = strings.ToUpper(name)
		} else if strings.HasPrefix(low, "f") {
			name = "F" + name[1:]
		}
		k, ok := keyCodes[name]
		if !ok {
			return nil, fmt.Errorf("unknown key %q in %q", part, s)
		}
		out = append(out, k)
	}
	if len(out) == 0 {
		return nil, errors.New("empty key chord")
	}
	return out, nil
}

// typedKey is the physical key, and whether Shift is held, that gives one character.
type typedKey struct {
	sc    uint32
	shift bool
}

// typeTables give, per macOS keyboard layout ID, the key for each character. Dead keys (ABNT2's
// ´ ` ~ ^ ¨) and the ABNT2 "/?" key, which does not reach macOS, are left out, so TypeKeys
// refuses those characters instead of typing others.
var typeTables = map[int]map[rune]typedKey{0: {}, 128: {}}

func init() {
	add := func(t map[rune]typedKey, plain, shifted []rune, codes ...uint32) {
		for i, sc := range codes {
			if plain[i] != ' ' {
				t[plain[i]] = typedKey{sc, false}
			}
			if shifted[i] != ' ' {
				t[shifted[i]] = typedKey{sc, true}
			}
		}
	}
	digits := []uint32{0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b}
	us, br := typeTables[0], typeTables[128]
	add(us, []rune("1234567890"), []rune("!@#$%^&*()"), digits...)
	add(us, []rune("-=[]\\;',./`"), []rune("_+{}|:\"<>?~"), 0x0c, 0x0d, 0x1a, 0x1b, 0x2b, 0x27, 0x28, 0x33, 0x34, 0x35, 0x29)
	add(br, []rune("1234567890"), []rune("!@#$% &*()"), digits...)
	add(br, []rune("-=[]ç';,.\\"), []rune("_+{}Ç\":<>|"), 0x0c, 0x0d, 0x1b, 0x2b, 0x27, 0x29, 0x35, 0x33, 0x34, 0x56)
	br['/'] = typedKey{0xb5, false} // the keypad's
	for _, t := range []map[rune]typedKey{us, br} {
		for c := 'a'; c <= 'z'; c++ {
			sc := keyCodes[string(c-'a'+'A')].Scancode
			t[c], t[c-'a'+'A'] = typedKey{sc, false}, typedKey{sc, true}
		}
		t[' '], t['\t'] = typedKey{0x39, false}, typedKey{0x0f, false}
	}
}

// TypeKeys turns text into key chords for the macOS layout id (see RecoveryKeyboard); it never
// adds Return. Layouts without a table, and characters a table lacks, are refused.
func TypeKeys(text string, layout int) ([][]KeyCode, error) {
	t, ok := typeTables[layout]
	if !ok {
		return nil, fmt.Errorf("typing text is supported only with the U.S. and Brazilian ABNT2 layouts (this VM uses layout %d)", layout)
	}
	var out [][]KeyCode
	for _, r := range text {
		k, ok := t[r]
		if !ok {
			if r == '\n' || r == '\r' {
				return nil, errors.New("text cannot contain a line break: send the Return key with send_keys")
			}
			return nil, fmt.Errorf("cannot type %q with this keyboard layout", r)
		}
		kc := KeyCode{Scancode: k.sc, Keysym: uint32(r)}
		if k.shift {
			out = append(out, []KeyCode{keyCodes["LeftShift"], kc})
		} else {
			out = append(out, []KeyCode{kc})
		}
	}
	return out, nil
}

// GuestLayout is the macOS layout ID the recovery uses on this host (see RecoveryKeyboard).
func GuestLayout() int {
	return macKeyboardID(hostKeyboard())
}

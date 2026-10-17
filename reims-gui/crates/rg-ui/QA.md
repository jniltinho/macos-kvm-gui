# rg-ui visual QA checklist

Run the demo (`cargo run -p rg-ui --example demo`) at 100% and 200% scale, in Light and Dark, and
compare with `docs/screenshots/*.png` and `tests/snapshots/*.png`.

## Layout and type
- [ ] Menu bar, toolbar (New, Import | Settings, Remove | Start, Shut down, Force stop | Window, Full screen), state chip at the right, VM list, tab strip, status bar: same order as the Go app.
- [ ] Spacing on the 4/8 px grid; body text 13 px Noto Sans, captions 11 px, commands in JetBrains Mono.
- [ ] Section bands 28 px, header color, colored icon, bold title; key/value rows tight (18 px).
- [ ] No tofu boxes (□) anywhere: check arrows and symbols in every tab and dialog.
- [ ] Long names and paths truncate or wrap; nothing overflows its card at 960 px wide.

## Icons
- [ ] Toolbar and section icons crisp at 200% (SVG rasterized at the pixel size).
- [ ] Lucide icons follow the theme (tab strip, buttons, badges); disabled toolbar icons faded.

## States
- [ ] Every button, tab, list row and settings page shows hover, pressed and disabled states.
- [ ] Tab moves focus with a visible 2 px focus ring; Shift+Tab goes back; Enter activates.
- [ ] Tooltips on toolbar buttons (with shortcuts), tabs (Ctrl+1–6), list rows and Host buttons.
- [ ] Menus show shortcuts: Ctrl+N, Ctrl+I, Ctrl+E, Ctrl+S, Ctrl+, , Ctrl+Enter, Ctrl+H, Ctrl+Delete, F11.
- [ ] Empty states with an icon: no VMs, no VM selected, empty log, no GPU.
- [ ] Busy task: spinner and stage in the status bar; Install progress line animates.
- [ ] Task done: toast slides in at the bottom right and fades out; errors open a dialog.
- [ ] Running VM: green state chip with a pulsing dot; the app idles (no repaint) when stopped.

## Dialogs
- [ ] Settings, New VM, Preferences, confirmations: backdrop dim, 8 px radius, layered shadow.
- [ ] Enter is OK/Finish, Esc is Cancel; the top dialog only reacts.
- [ ] Settings › Display: Shown disabled for Reims, Resolution only for Reims, the wide-resolution warning on Sequoia, METAL_ERROR_MODE commands with working Copy buttons.
- [ ] Settings › Network: Adapter 1–4 tabs, bridge name only when bridged, driver warnings.
- [ ] Wizard: one section open at a time; a validation error opens the faulty section.

## Screen tab (design D3)
- [ ] The rectangle sent to the backend matches the tab in physical pixels at 100% and 200%.
- [ ] `overlay_open` turns on with any dialog, menu or combo popup and off when it closes.
- [ ] Switching to another tab sends no rectangle (the embedded window unmaps).
- [ ] Live (X11 and XWayland): a Reims VM and an SVGA VM start inside the tab with no window left on the desktop; the guest follows tab resizes and guest mode changes; a menu, combo or dialog over the tab hides it and clicks never reach the guest; the pointer entering the guest gives it the keyboard, a click on the app takes it back; ABNT2, keypad Enter and keypad Divide reach macOS; restart re-embeds; quitting the app leaves an outside VM's window on the desktop.
- [ ] Window puts QEMU's window back on the desktop and "Show it here" brings it back; Full screen slides the shell away and Exit full screen (or F11) restores it.
- [ ] Wayland without X server: Reims says its window is on the desktop; SVGA shows the VNC screen, Ctrl+Alt releases the keyboard.

## File dialogs
- [ ] Browse… opens the portal's chooser in front of the app; without a portal the dialog says so and the path can be typed.
- [ ] Export (full) and Import (.tar) show the progress dialog; Hide keeps the task running in the status bar.

## Contrast
- [ ] `theme::tests::text_reaches_aa_contrast_on_every_surface` passes (4.5:1 for text, captions and state colors on every surface, both themes).

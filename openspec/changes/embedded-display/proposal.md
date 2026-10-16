# Proposal

## Why

The VM's screen opens in a separate QEMU window, so the app and the VM live in two places, and stopping or checking the VM means switching windows. Showing the screen inside the app's VM tab, with keyboard and mouse working, makes the app the single place to use the VM.

## What Changes

- QEMU exports the VM's screen over VNC on a private Unix socket next to the QMP socket (no TCP port), and its own window is turned off.
- The VM tab shows the live screen with keyboard and mouse input, scaled to fit while keeping the aspect ratio, and can open it in a separate, resizable window.
- A new **Display** setting chooses between **In the app** (default) and **QEMU window** (the previous behavior), as a fallback.
- Out of scope: clipboard sharing, audio, non-US keyboard layouts beyond what QEMU's `en-us` keymap maps, and the passed-through GPU's own outputs (they still go to the monitor plugged into the card).

## Capabilities

### New Capabilities
- `embedded-display`: VNC export from QEMU on a private socket and the in-app viewer with input.

### Modified Capabilities
<!-- None: the VM start behavior is unchanged when the Display setting is "QEMU window". -->

## Impact

- `internal/core`: a minimal RFB (VNC) client, QEMU arguments (`-display none -vnc unix:…`), and a `display` config field.
- `main.go`: the screen widget (rendering, scaling, keyboard and mouse), the detach window, and the Display setting.
- No new dependencies: the RFB client uses only the standard library.

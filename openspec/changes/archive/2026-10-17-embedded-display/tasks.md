# Tasks

## 1. Core

- [x] 1.1 Add the `display` config field (`app` default, `window`) with validation; verify with config tests
- [x] 1.2 Add `-display none -vnc unix:<private dir>/vnc.sock` for `app` and keep the old arguments for `window`; verify with argument tests
- [x] 1.3 Implement the RFB client (handshake, None security, pixel format, Raw, CopyRect, DesktopSize, key and pointer events); verify against a fake server over net.Pipe
- [x] 1.4 Expose the VNC socket path from the running VM; verify with the real-QEMU integration test receiving a non-empty first frame

## 2. Interface

- [x] 2.1 Screen widget: rendering at up to 60 fps, contain-fit scaling, keyboard, mouse, wheel and focus handling with key release on focus loss; verify the coordinate mapping and key mapping with unit tests
- [x] 2.2 VM tab integration, placeholder when stopped, and "Open in window"; verify with screenshots of a running VM
- [x] 2.3 Display setting in the Settings tab; verify switching to "QEMU window" restores the old window
- [x] 2.4 Full screen button and the Ctrl+Alt+F host key; verify entering and leaving full screen and returning to the tab on a real VM
- [x] 2.5 Hide the host pointer over the screen, redraw at up to 60 fps and drop repeated pointer events; verify a single pointer on a real VM

## 3. Documentation and integration

- [x] 3.1 Document the embedded display, its keyboard limits and the fallback in docs/ and the README; verify links
- [x] 3.2 Run `make test`, the integration test and `openspec validate --all --strict` — 2026-10-17: `make test`, `make test-integration` (TestRealQEMULifecycle got a 1280x800 VNC frame) and `openspec validate --all --strict` (13/13) pass.

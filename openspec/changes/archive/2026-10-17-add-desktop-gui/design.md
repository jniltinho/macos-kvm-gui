# Design

## Context

OSX-KVM ships shell scripts (`OpenCore-Boot.sh` with hardcoded values), a Python downloader (`fetch-macOS-v2.py`) and the boot binaries (OpenCore qcow2, OVMF). On the reference host (Ubuntu 24.04, Ryzen 9 9900X, QEMU 8.2.2, OSX-KVM `4c378a4`) the manual flow went like this: `qemu-img convert` converted the Sonoma DMG without `dmg2img`; the `Skylake-Client,-hle,-rtm` CPU booted Sonoma; and a running VirtualBox VM made QEMU fail with `KVM_CREATE_VM ... busy`.

A review found a problem in `fetch-macOS-v2.py`: in the `-s` flow, `verify_image()` calls `os.get_terminal_size()` without handling a missing TTY, and the return value of `action_download()` is discarded. So with stdout piped, verification fails and the process still exits with code 0. Requirements are in the specs; motivation is in proposal.md.

## Goals / Non-Goals

**Goals:**
- A Go binary that only invokes tools already present on the host (qemu, git, python3, pkexec).
- Testable logic (argument building, validation, checks, chunklist, QMP) separated from the UI.
- Never destroy an existing disk, media or NVRAM as a side effect.

**Non-Goals:**
- Reattaching to a VM started outside the app or in a previous run.
- Updating the OSX-KVM clone (`git pull`) or pinning a revision of it.
- Reimplementing Apple's recovery protocol in Go; only chunklist verification.
- Partitioning the disk or automating the installer inside the guest.
- Packaging (.deb, Flatpak, AppImage) and initramfs regeneration.

## Decisions

1. **Fyne v2 as the toolkit.** Pure Go widgets, single binary, no web runtime. Long operations run in goroutines and update widgets via `fyne.Do` (Fyne ≥ 2.6). Rejected alternatives: Wails (webkit2gtk plus a JS frontend, i.e. two stacks) and gotk4 (heavy GTK bindings). Cost: CGO and OpenGL/X11 headers at build time.
2. **Reuse OSX-KVM at runtime instead of embedding it.** The app clones the repository into the workspace and uses `OpenCore.qcow2`, OVMF and `fetch-macOS-v2.py` from there, without redistributing GPLv3 or Apple binaries. A non-empty directory is used only if it is complete; otherwise nothing is changed.
3. **QEMU arguments generated in Go.** `OpenCore-Boot.sh` is ported to a pure function `config → []string`, covered by table tests. In QEMU options, commas in paths are escaped as `,,`. OpenCore uses `snapshot=on`. SSH uses `hostfwd=tcp:127.0.0.1:<port>-:22`. The topology is `-smp <vcpus>,sockets=1,cores=<c>,threads=<vcpus/c>`.
4. **Hardware profile derived from the version**, following upstream comments: Penryn up to Monterey, Skylake-Client from Ventura on, vmxnet3 for High Sierra. This removes options from the UI and the risk of invalid combinations. Only Sonoma has been validated; the other versions are shown as experimental.
5. **Persistent NVRAM per workspace.** On first start, `OVMF_VARS-1920x1080.fd` is copied to `macos-kvm-gui-vars.fd`, which is never overwritten. There is no resolution option: the upstream VARS and OpenCore already ship at 1920x1080, and switching VARS would lose the NVRAM.
6. **Download via `fetch-macOS-v2.py`, verification in Go.** The script runs with `-s <version>` inside `<workspace>/.macos-kvm-gui-staging/`, and its exit code does not define success. Go reads the chunklist (`CNKL` header, RSA-2048 signature with Apple's EFI public key, same as the script), checks the SHA-256 of each DMG chunk and rejects trailing bytes. Progress comes from output split on `\r` or `\n`, parsing `NN.N% downloaded`; the stage is inferred from the `Saving ...chunklist` and `Saving ...dmg` lines.
7. **Atomic publish.** `qemu-img convert -p -f dmg -O raw` writes to staging, on the same filesystem as the destination. Before the rename, the old version marker is deleted; then a `rename` replaces `BaseSystem.img`. The rename is the commit point: before it, any failure preserves the previous media. After it, writing `.macos-kvm-gui-media` is best-effort, and a failure there only leaves the version as "unknown", never a wrong version. Staging is always removed. Before starting, the app requires 4 GiB free.
8. **Per-workspace lock with `flock`** at `$XDG_RUNTIME_DIR/macos-kvm-gui-<sha256(path)[:8]>.lock`. The lock lives **outside** the workspace, because inside it the directory would no longer be empty and `git clone` would refuse. It is held during any operation that changes the workspace and for the whole VM run; a second instance gets "workspace in use". The kernel releases the lock if the app dies. The configuration is copied at the start of each operation.
9. **QMP with a minimal in-house client.** The socket lives in `os.MkdirTemp` (mode 0700) under `$XDG_RUNTIME_DIR`, or the temp directory if it does not exist. The client tries to connect up to 50 times, every 100 ms, until QEMU creates the socket. It then reads the greeting, sends `qmp_capabilities` and keeps a single connection for the run. A reader goroutine separates events (`event` field) from responses (`return`/`error`). Each command has 5 s to write and receive the response. `BLOCK_IO_ERROR` with `action: "stop"` or a `STOP` event changes the state to "Paused (error)" (the app never pauses the VM on its own), and `RESUME` returns it to "Running". If the QMP connection drops while the process is alive, the state stays "Running", graceful shutdown becomes unavailable and force stop remains available. `system_powerdown` does not change the state.
10. **Elevation only for `ignore_msrs`.** A constant script, with nothing interpolated, runs via `pkexec /bin/sh -c '...'`. It writes `/sys/module/kvm/parameters/ignore_msrs` and `/etc/modprobe.d/macos-kvm-gui.conf`, a file of its own. pkexec exit code 126 means cancelled; any other non-zero code is an error. Afterwards, the app rereads both states to detect partial application. Persistence covers kvm loaded as a module; initramfs is out of scope and the README documents this.
11. **Hypervisor conflict as a warning.** The app looks for `VBoxHeadless`, `VirtualBoxVM` and `vmware-vmx` in `/proc/*/comm`, and a read error becomes "Inconclusive". The definitive diagnosis is the QEMU error at start (decision 9 and the error translation).
12. **JSON configuration** at `os.UserConfigDir()/macos-kvm-gui/config.json`, written atomically (temp file and rename).
13. **Code layout:** `main.go` (Fyne) plus `internal/core` (config, host, media, chunklist, qmp, vm), with tests in the same package. For testing, `/proc` and `/sys`, the command executor and the QMP socket are injectable.

## Risks / Trade-offs

- [Upstream changes file names or the downloader output] → Paths are constants, chunklist verification is independent of the script and, if progress parsing fails, the bar becomes indeterminate.
- [App killed while the VM is running] → QEMU is a child of the app and receives SIGTERM via `Pdeathsig`. The loss is equivalent to a force stop, but it avoids an orphan QEMU holding the workspace without a lock. Since `Pdeathsig` fires when the **thread** that created the process dies, `Start` and `Wait` run in a goroutine with `runtime.LockOSThread`, and that thread lives as long as QEMU does.
- [Experimental versions fail with the derived profile] → They are marked experimental in the UI; preflight checks AVX2/SSE4.1 from Ventura on.
- [qemu-img does not support some DMG's compression] → Conversion fails safely (previous media preserved) and the error suggests installing `dmg2img`.
- [Thin qcow2 exhausts the host disk] → Warning at creation and "Paused (error)" state from QMP.
- [CGO build complicates distribution] → Accepted for this phase; the README lists the packages.

## Migration Plan

Not applicable: new project. An existing OSX-KVM workspace (`~/OSX-KVM` with disk and media) is reused, because the file names are the upstream ones. The app's own NVRAM is created on the first start done by the app.

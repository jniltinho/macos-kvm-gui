# Proposal

## Why

Without a passed-through GPU, macOS draws on the CPU (VMware SVGA has no macOS driver), and passthrough needs a spare, macOS-supported card, VFIO and a host reboot. Reims vGPU (steelbrain/reims-vgpu, GPL-2.0-or-later or LGPL-3.0-or-later) is a paravirtual GPU that gives macOS guests Metal acceleration through the host's Vulkan driver, with the GPU driver macOS already ships: it works with the host's own display GPU (NVIDIA, AMD, Intel), including cards macOS cannot drive, such as the reference host's RTX 4070 and RX 6700 XT.

## What Changes

- New per-VM **Graphics** setting: `vmware-svga` (default, today's behavior) or `reims-vgpu` (experimental). It is offered for every macOS version, with a note that upstream measured x86-64 guests on macOS 26 only.
- The app does not ship Reims vGPU. It uses a Reims QEMU build in `<Workspace>/.reims/qemu/current`, next to the app's other shared data (`.kexts`, `.installers`), or `reims_qemu` in `settings.yaml`, or an upstream install in `~/.local/share/reims-ctl/qemu/current`, and runs that QEMU only for VMs with `reims-vgpu`.
- **Prebuilt packages as release assets.** A new workflow (`reims-release.yml`, run on a `reims-<commit>` tag) builds upstream's QEMU at a pinned commit for Ubuntu 24.04 and 26.04 in Docker, strips it and publishes `reims-qemu-<commit>-ubuntu<release>-x86_64.tar.zst` (about 30 MB each), their SHA-256 files and the corresponding source (`reims-vgpu-src-<commit>.tar.zst`, with QEMU's source and the vendored Rust crates) on this repository's Releases. The app embeds a manifest of that release (tag, file names, SHA-256) and installs the package for the host's release from the Host tab's **Install Reims vGPU** button or `reims install`: download, verify, unpack into `<Workspace>/.reims/qemu/<build>`, check on the host, activate.
- As a fallback (another Ubuntu release, another upstream commit), the app embeds the same small build recipe (a Dockerfile and its steps, not binaries) and builds upstream's QEMU at a pinned commit, with upstream's own installer, inside an Ubuntu container of the host's release (24.04 or 26.04, the supported targets for now): a **Build Reims vGPU** button in the Host tab and `macos-kvm-gui reims build`, with progress in the log. The result is checked on the host before it is activated in `<Workspace>/.reims/qemu/current`. Nothing else is installed on the host. `reims status` and the Host tab show what is installed.
- With `reims-vgpu`, the QEMU command follows upstream's measured x86-64 command line: `apple-gfx-pci` in place of `vmware-svga`, guest RAM as a shared memfd, no ACPI S3, and a USB audio device (macOS 26 has no Intel HDA driver).
- The screen is Reims vGPU's own accelerated window. The app's VNC display stays for the Preview, the MCP server's screen capture and key input, but is not the VM's screen.
- Host checks for the mode: the Reims QEMU build with `apple-gfx-pci`, a Vulkan driver, `vm.max_map_count` large enough for the VM's RAM, and on AMD GPUs access to `/dev/udmabuf`.
- Reims vGPU and GPU passthrough are mutually exclusive per VM.
- Guest setup guidance: `METAL_ERROR_MODE` 5, set once (from Recovery on a new install), without which the desktop does not come up.
- CLI `config set graphics=…`, the Settings dialog's Display page, the Details pane, docs.

## Capabilities

### New Capabilities
- `reims-vgpu`: the per-VM paravirtual GPU option, its QEMU build discovery, host checks, QEMU command line, display and guest guidance.

### Modified Capabilities

## Impact

- `internal/core`: `config.go` (Graphics field, validation, exclusivity with GPU), `vms.go` (schema 4: `graphics` column), `vm.go` (`BuildArgs`, QEMU binary choice, environment), `host.go` (checks), `settings.go` (`reims_qemu` key), new `reims.go` (discovery, `-device help` probe, `vm.max_map_count`, udmabuf).
- `internal/gui`: Settings › Display (Graphics choice), Details, Screen tab and Preview, Host tab checks and the build button.
- `cmd/config.go` (`graphics` key), new `cmd/reims.go` (`reims install`, `reims build`, `reims status`), `internal/mcp` unchanged (it starts VMs through the CLI).
- Docs: `docs/reims-vgpu.md` (new), `docs/architecture.md`, `docs/troubleshooting.md`, `docs/cli.md`, `docs/references.md`.
- New `internal/core/reims/Dockerfile` and `internal/core/reims/manifest.json` (embedded); new `.github/workflows/reims-release.yml`; `THIRD_PARTY_NOTICES.md`. Docker is needed only to build Reims vGPU from source, not to install a package.
- No new Go dependency. Reims vGPU itself stays an external program the user builds.

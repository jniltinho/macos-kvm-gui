# AGENTS.md

Guidance for AI coding agents (Claude Code, Codex, Cursor) working in this repository.

## Two apps, one repository

| App | Language | Where | Releases (tag → title) | Status |
|---|---|---|---|---|
| **reims-gui** | Rust 1.99, egui/eframe 0.36 | `reims-gui/` (Cargo workspace) | `reims-gui-vX.Y.Z` → `reims-gui (Rust) vX.Y.Z` | Recommended; marked **latest** |
| **macos-kvm-gui** | Go 1.27, Fyne 2.8 | `main.go`, `cmd/`, `internal/` | `vX.Y.Z` → `macos-kvm-gui (Go) vX.Y.Z` | Maintained; never marked latest |
| Reims vGPU's QEMU | upstream steelbrain/reims-vgpu | `internal/core/reims/`, `reims-gui/crates/rg-reims/reims/` | `reims-<commit>` → `Reims vGPU QEMU <commit>` | Packages the apps install |

Both apps read the same VM list and settings (`~/.config/macos-kvm-gui/`), so they can be installed side by side.

## Rules

- **English only** in code, comments, docs, UI text and commit messages. Talk to the user in their language.
- **Validate locally before GitHub.** Build, test and check packages here first, then push or tag.
- **Docker-only Rust builds.** Never install Rust toolchains or build dependencies on the host. Use `reims-gui/scripts/cargo-docker.sh [24.04|26.04] <cmd>`.
- **Keep the last 2 releases and tags of each series** (`v*`, `reims-gui-v*`, `reims-*`). Delete older ones with `gh release delete <tag> --cleanup-tag -y`.
- **Release notes** follow `.claude/skills/create-release/SKILL.md`:
  - the home lab notice;
  - a note saying which app the release is (Go or Rust; recommend reims-gui);
  - ✨/🔧/🧹/📚 sections, Install, **Validated** (only what really ran) and Full Changelog;
  - Full Changelog links must point at tags that still exist, or at commit ids.
- **OpenSpec** (`openspec/`) for features: propose, have Codex review it (`codex exec -s read-only -C . "<review prompt>"`), fix P1/P2 findings, implement, mark each task with its evidence, archive. `openspec validate --all --strict` must pass.
- **Secrets.** The test VM's password is never written to files, commits or docs.
- **Destructive or outward actions** (force pushes, deleting releases outside the 2-per-series rule, posting to third-party repos) need the user's explicit OK.
- **The user's desktop.** Do not drive it with xdotool while the user is using it. Ask first, or use the GNOME containers.

## Build and test

### reims-gui (Rust), from `reims-gui/`

```sh
scripts/cargo-docker.sh 24.04 sh -c 'cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked'
scripts/build.sh                 # one .deb + .tar.gz, built on 24.04, in dist/
scripts/check-package.sh 24.04   # and 26.04: apt install + --self-test in a clean container
scripts/check-gnome.sh 24.04     # and 26.04: under Mutter (Wayland + XWayland), SVGA embed, SIGTERM release
```

- **One package for every release.** It is built on Ubuntu 24.04, the oldest supported glibc (2.39). It installs on Ubuntu 24.04, 26.04 and Debian 13; the `.tar.gz` runs on Rocky Linux 10.
- **No plain binary in releases.** The `.deb` depends on `qemu-system-gui` (QEMU's GTK window, embedded for VMware SVGA VMs). Without it, the app falls back to its VNC screen.
- **`unsafe` is forbidden** in the workspace, except in `crates/rg-x11-ffi` (the C ABI for the Go app).

### macos-kvm-gui (Go), from the repository root

```sh
make test            # gofmt, go vet, go test -race (no Docker, no Rust)
make test-embed      # internal/embed's cgo test under Xvfb (needs Xvfb, xdotool, xwininfo)
make build           # dist/macos-kvm-gui (Go, -tags reims_embed) and dist/reims-gui (Rust)
make package         # both apps' packages in dist/ (package-go: .deb, .rpm, .tar.gz)
make test-integration  # real QEMU/KVM boot of the embedded OpenCore
```

- **Embedding Reims vGPU's window in the Go app.**
  - `internal/embed` uses cgo over `reims-gui/crates/rg-x11-ffi` (`rg_x11.h`), behind the build tag `reims_embed`; plain `go build .` uses a stub.
  - `make embed-lib` builds the static library in Docker and prunes it with `ld -r --gc-sections`, keeping only the `rgx_*` roots. Otherwise Rust std's weak `pidfd_*` symbols raise the Go binary's glibc floor to 2.39. The floor must stay **GLIBC_2.38** (the `.deb` and `.rpm` declare Fedora 39 and Debian 13). Check it with `objdump -T dist/macos-kvm-gui | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1`.
- `go version -m dist/macos-kvm-gui` must show `-tags=reims_embed` for shipped builds.

### Reims vGPU's QEMU

- **Ubuntu and Debian:** `docker build --build-arg UBUNTU=24.04|26.04` (or `--build-arg BASE=debian:13`) `-t reims-build:<rel> - < internal/core/reims/Dockerfile`, then `docker run ... -v <dir>:/build reims-build:<rel>`.
- **Rocky Linux 10:** `internal/core/reims/Dockerfile.rocky`.
- An Ubuntu build does not run on Debian (`libjpeg.so.8`); each distribution needs its own.
- **What the apps install:** "Install Reims vGPU" and `reims install` fetch only the Ubuntu packages that the manifest pins (`internal/core/reims/manifest.json`, `reims-gui/crates/rg-reims/reims/manifest.json`).
- **Nothing installs it on its own:** starting a Reims VM does not download QEMU. macOS Tahoe's `ReimsVgpu.kext` is the only part fetched automatically.

## Workflows (`.github/workflows/`)

| File | Runs | Does |
|---|---|---|
| `reims-gui-ci.yml` | push and PR on `reims-gui/**` | fmt, clippy, tests; one package checked on 24.04 and 26.04 (apt and GNOME) |
| `reims-gui-release.yml` | tag `reims-gui-v*` (must equal `reims-gui/Cargo.toml`'s version) | builds, checks, publishes **latest** |
| `go-ci.yml` | manual (`gh workflow run go-ci.yml --ref main`) | `make test`, `make build-prod`, `openspec validate` |
| `go-release.yml` | manual on a `v*` tag (`gh workflow run go-release.yml --ref vX.Y.Z`) | `make package-go`, publishes, not latest |
| `reims-release.yml` | tag `reims-<commit>` | Reims QEMU for 24.04, 26.04, debian13, rocky10 and the source archive |

## Testing on real hardware

- **Reference host:** Ubuntu 24.04, KDE Plasma on X11, Ryzen 9 9900X, RTX 4070 (Reims vGPU validated) and RX 6700 XT.
- **Test VM:** "Nilton" (macOS Sequoia 15.8.1, Reims vGPU), SSH at `127.0.0.1:2222`, user `nilton`; the password comes from the user, never stored. Shut it down over SSH (`sudo shutdown -h now`), not with Force stop.
- **Do not run two QEMUs on the same disk.** The live GNOME containers (`rg-live-img:24.04` / `26.04`) use a qcow2 overlay over the host disk mounted read-only; do not start the VM on the host while they run.

## Docs to keep in step

`README.md` (leads with reims-gui), `reims-gui/README.md`, `docs/reims-gui.md`, `docs/reims-vgpu.md`, `docs/development.md`, `docs/architecture.md`, `docs/screenshots/` (captured from the real apps), `reims-gui/PARITY.md`, `THIRD_PARTY_NOTICES.md`.

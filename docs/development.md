# Development

[Documentation index](README.md)

## Build

Use the Go version declared in [`go.mod`](../go.mod) (**1.27.2**), CGO, Make and Fyne's native build dependencies. Debian/Ubuntu setup:

```sh
sudo apt install make gcc pkg-config libgl1-mesa-dev xorg-dev \
  libwayland-dev libxkbcommon-dev wayland-protocols qemu-utils \
  curl unzip
make build          # dist/macos-kvm-gui, development version
make run            # build and launch
make build-prod     # stripped, -trimpath, main.version from VERSION
```

Boot assets are committed; a normal build does not fetch them. An empty Go module cache may still require network access for dependencies. Running the app also needs `qemu-system-x86` and `pkexec`; see [requirements](getting-started.md#requirements).

`make package VERSION=v0.X.Y` builds the production binary and calls `scripts/package.sh`, producing a Linux-amd64 `.tar.gz`, `.deb` and one `.sha256` per artifact in `dist/`. Packaging needs `dpkg-deb`, `tar` and `sha256sum`. The tarball includes the binary, README and licenses; the `.deb` installs the binary, desktop entry and SVG icon. It declares glibc 2.38+, desktop runtime libraries and QEMU/pkexec dependencies. This is not a cross-platform package target. `make clean` removes `dist/`.

## Tests and validation

```sh
make test
```

This first fetches `internal/core/testdata/ocvalidate` if absent, using `scripts/fetch-ocvalidate.sh`, then checks gofmt, runs `go vet ./...` and `go test -race ./...`. Fetching verifies the OpenCore release checksum from `assets.env` with the same environment/repository `.env` overrides. The validator is git-ignored and not embedded. With it already present and Go dependencies cached, the unit-test path needs no downloads.

Tests cover configuration/profiles, fake host/sysfs checks, reservation scripts, QEMU arguments, QMP with a fake server, recovery with a local HTTP server, real Apple chunklist signature data and synthetic signed-image failures. Disk/media tests use real `qemu-img`; those tests skip when it is missing. They do not download a full macOS installation or exercise physical GPU passthrough.

`TestOpenCoreConfig` generates install, installed and GPU config variants, invoking `ocvalidate` when the executable exists and requiring `No issues found`. Running bare `go test` without it does **not** validate the schema. To explicitly fetch/refresh it and run that test:

```sh
scripts/fetch-ocvalidate.sh
go test -race -run '^TestOpenCoreConfig$' ./internal/core
```

### Integration with real QEMU/KVM

Install `qemu-system-x86`, ensure `/dev/kvm` access and a working graphical environment for QEMU's default display, and stop conflicting hypervisor VMs:

```sh
make test-integration
# go test -race -tags integration -run Real ./internal/core
```

`TestRealQEMULifecycle` creates a temporary workspace (including commas in its path), creates a 40 GiB thin disk, boots embedded OpenCore via vvfat, checks persistent VARS creation and lock ownership, sends QMP powerdown, then force-stops QEMU and checks cleanup. It uses host SSH port **22299**. It does **not** install macOS or prove a guest reaches its desktop.

The integration-tagged `TestPrintBootArgs` is a separate manual helper: with `MKG_BOOT_WS` set, it prepares that workspace and writes NUL-separated QEMU arguments to `<workspace>/args`. It is excluded by `-run Real`. Use a disposable workspace/overlay, not an unprotected production guest disk, for manual boot validation.

Only Sonoma has recorded end-to-end validation in the repository. GPU passthrough is **untested on real hardware**. Report unit, QEMU-lifecycle, guest-desktop and real-GPU validation separately.

## Updating boot assets

[![Assets pipeline](diagrams/assets-pipeline.png)](diagrams/assets-pipeline.svg)

1. Review upstream release licenses and record the version, canonical download URL and independently checked SHA-256 in [`assets.env`](../assets.env). Maintain [third-party notices](../THIRD_PARTY_NOTICES.md) if needed.
2. Install `curl`, `sha256sum`, `unzip` and `dpkg-deb`, then run `scripts/fetch-assets.sh`. It downloads all components and verifies every checksum before deleting/rebuilding `internal/assets/files`. Hash failure preserves the old tree; a later extraction failure is not an atomic update.
3. Review the rebuilt EFI tree, OVMF pair and `files/VERSIONS`. The script also refreshes test-only `ocvalidate`. Commit the intended boot files, not the validator or generated build output.
4. If OpenCore's schema changed, update `internal/assets/config.plist.tmpl` from the corresponding upstream sample, retaining attribution and KVM settings. A version bump alone does not update the template.
5. Run `make test`, `make test-integration` where KVM is available, and a real macOS boot with a disposable overlay. Check picker/media behavior, resolution, kexts, stable identity and unchanged NVRAM.

The scripts parse plain `KEY=value` data, never source shell files. Only component `VERSION`, `URL`, `SHA256` keys are accepted (the validator-only script reads OpenCore URL/SHA256). Precedence is **process environment > repository `.env` > `assets.env`**. Keep repository `.env` values unquoted and without shell expressions.

For a runtime experiment without rebuilding:

```sh
MKG_ASSETS_DIR="$PWD/internal/assets" ./dist/macos-kvm-gui
```

This is a complete-tree override, not a per-file overlay. The Install tab reports its versions/source. It is separate from the optional runtime `.env` under the user config directory; see [architecture](architecture.md#settings-and-overrides). Boot files extracted into a workspace are regenerated on every Start, so do not use workspace edits as the development source.

## OpenSpec workflow

[`openspec/config.yaml`](../openspec/config.yaml) selects the `spec-driven` schema and English artifacts/UI/comments. Changes live under [`openspec/changes/`](../openspec/changes/), with `proposal.md`, `design.md`, `specs/<capability>/spec.md`, `tasks.md` and `.openspec.yaml`.

- Explain the motivation/scope in the proposal, decisions and risks in the design, and behavior as requirements/scenarios in spec deltas.
- Implement and verify task-list items; keep validation evidence and untested work visible.
- Validate all changes using the CI-pinned tool:

  ```sh
  npx -y @fission-ai/openspec@1.14.2 validate --all --strict
  # Or one change:
  npx -y @fission-ai/openspec@1.14.2 validate embed-boot-assets --strict
  ```

- Use OpenSpec's archive workflow after acceptance; `openspec/changes/archive/` is the archive destination. Validation checks spec structure, not runtime correctness.

Read changes in context: `add-desktop-gui` describes the original clone/Python design, now superseded by `embed-boot-assets`. Parts of the original GPU proposal also predate keeping `vmware-svga` and generating `agdpmod=pikera`. Current code and the later boot-assets design define those implemented behaviors; do not restore old assumptions from historical proposals.

## CI and releases

| Workflow | Trigger and work |
| --- | --- |
| [CI](../.github/workflows/ci.yml) | Pushes to `main` and pull requests. Ubuntu 24.04; Go from `go.mod`; install native headers and `qemu-utils`; `make test`, `make build-prod`. Separate Node 24 job validates OpenSpec 1.14.2 with `--all --strict`. New runs cancel earlier runs for the same ref. |
| [Release](../.github/workflows/release.yml) | Pushes of `v*` tags. Ubuntu 24.04, America/Sao_Paulo timezone, Go from `go.mod`, native dependencies, `make test`, then `make package VERSION=${GITHUB_REF_NAME}`. Publishes a GitHub Release with generated notes and `.tar.gz`, `.deb`, `.sha256` attachments. |

Neither workflow runs the real-QEMU integration target, a macOS installation or a real-GPU test. Release does not run the separate OpenSpec job. Builds use committed boot assets rather than refetching them; tests may fetch `ocvalidate` on a fresh runner.

### The create-release skill

The repository's [`.claude/skills/create-release/SKILL.md`](../.claude/skills/create-release/SKILL.md) documents the maintainer release procedure; it is not a Make target or automatic version bump. It requires authenticated `gh`, a clean/up-to-date `main`, fetched tags and passing local tests. It uses `v0.MINOR.PATCH`: feature commits bump MINOR; fixes/improvements/docs/chores alone bump PATCH.

The procedure reviews commits, creates an annotated `v*` tag, pushes it, watches `release.yml`, then replaces the generated notes with categorized New Features, Improvements, Cleanup and Documentation sections (omitting empty sections). Keep the home-lab/license notice, install instructions, actual hardware-validation scope and full changelog link. Finally check the release title, four attached artifacts, SHA-256 files and that the binary opens with the tagged title. Never describe unit coverage as real-GPU validation.

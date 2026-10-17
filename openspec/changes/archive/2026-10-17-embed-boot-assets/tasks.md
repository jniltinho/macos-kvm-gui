# Tasks

## 1. Assets

- [x] 1.1 Write `scripts/fetch-assets.sh` and `assets.env` (pinned URLs and SHA-256, env > .env > assets.env, no sourcing); verify a wrong checksum from the environment and from `.env` aborts without touching the built files
- [x] 1.2 Build and commit `internal/assets/files` and embed it with `go:embed`; verify `go build` works offline from the repository
- [x] 1.3 Derive `config.plist.tmpl` from OpenCore's `Sample.plist`; verify OpenCore 1.0.8 `ocvalidate` reports no issues

## 2. Boot

- [x] 2.1 Extract the EFI tree and OVMF, generate `config.plist` and the SMBIOS identity per workspace; verify with tests for extraction, stable identity, NVRAM never overwritten and the three config variants under `ocvalidate`
- [x] 2.2 Boot OpenCore from read-only `vvfat` on virtio-blk; verify with argument tests and the real-QEMU lifecycle test
- [x] 2.3 Add the `MKG_ASSETS_DIR` override (environment or `~/.config/macos-kvm-gui/.env`); verify with tests for the override, an incomplete directory and settings precedence
- [x] 2.4 Verify a real Sonoma boot with the embedded OpenCore on an overlay of an installed disk (no menu, iMac20,1, generated serial, 1920x1080, kexts loaded)

## 3. Recovery without Python

- [x] 3.1 Port `macrecovery` to Go and wire it into the media download; verify with a fake recovery server (cookies, POST body, tokens, cleanup on failure) and the existing verification and conversion tests

## 4. Interface and documentation

- [x] 4.1 Replace "Prepare workspace (clone OSX-KVM)" with "Create workspace", show the boot file versions and their source, drop the `git` and `python3` checks and show the home lab notice in the Host tab; verify with a screenshot
- [x] 4.2 Add `LICENSE`, `THIRD_PARTY_NOTICES.md` and README sections (requirements, boot assets, overrides, home lab notice, migration from OSX-KVM); verify the README renders

## 5. Integration

- [x] 5.1 Run `go vet ./...`, `go test -race ./...` and `openspec validate --all --strict` without errors

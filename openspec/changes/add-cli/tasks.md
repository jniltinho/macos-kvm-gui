# Tasks

## 1. CLI

- [x] 1.1 Move the GUI to internal/gui and add the Cobra root (GUI by default, `--config`); verify `macos-kvm-gui --help` and that the GUI opens without arguments
- [x] 1.2 Add check, setup, config, workspace, download, disk, media, vm, gpu and version commands; verify by running each on the reference host
- [x] 1.3 Add tests for config set/show, validation errors, `media remove` without `--yes` and `version`; verify `go test ./cmd`

## 2. Documentation

- [x] 2.1 Document the commands and an automated install script in docs/cli.md; verify the documented commands run

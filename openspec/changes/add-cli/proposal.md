# Proposal

## Why

Everything the app does is reachable only through the window, so setting up a host or a VM cannot be scripted. A command line over the same core lets users automate installs, run the VM on headless hosts and use the app in scripts and CI.

## What Changes

- Standard Cobra layout: `main.go` calls `cmd.Execute()`, with one file per command in `cmd/`. The desktop interface moves to `internal/gui`.
- Without a command the binary opens the GUI, so the menu entry and existing usage keep working.
- Commands: `check`, `setup`, `config show|set|recommend`, `workspace create`, `download`, `disk create`, `media remove --yes`, `vm start`, `gpu list|reserve|release`, `version` and `gui`, plus a global `--config`.
- Out of scope: managing several VMs, attaching to a VM started by another process, and machine-readable output beyond `config show` (JSON).

## Capabilities

### New Capabilities
- `cli`: command line access to the host checks and setup, settings, workspace, media, disk, VM and GPU operations.

### Modified Capabilities
<!-- None: the GUI behaves as before. -->

## Impact

- New dependency: `github.com/spf13/cobra`.
- Code moves: `main.go` → `internal/gui`; new `cmd/` package. The build-time version stays `main.version`.

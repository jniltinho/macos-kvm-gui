# Proposal

## Why

The app managed one VM through one `config.json`, and its "workspace" was that VM's folder. Labs need several macOS versions side by side, a guided way to create them, a tidy place for their files, and a way to move a VM to another host.

## What Changes

- The VM list and app preferences move to SQLite (`~/.config/macos-kvm-gui/macos-kvm-gui.db`, pure-Go `modernc.org/sqlite`). An older `config.json` is imported once and renamed to `.migrated`.
- The **Workspace** becomes the app-wide parent folder (default `~/macOS-VMs`). Each VM has its own folder, by default `<Workspace>/<name>`. **Move** relocates a VM's own files into a new folder.
- A four-step **New VM wizard**: name and macOS version, hardware recommended for the host, storage (folder, disk size, download now), summary. Create makes the folder and disk and can download the recovery.
- Edit, Delete (files are a separate, confirmed choice), **Export/Import** as a `.tar` bundle (settings, disk, NVRAM, identity, install media) or settings-only `.json`.
- Running detection through `/proc`, including QEMU started outside the app; Start refuses a disk already in use. One VM runs at a time.
- Theme selector (Dark, Light, System), applied at once and saved; `MKG_THEME` overrides it.
- CLI: `vm list|status|create|select|move|export|import|delete`, `workspace show|set`, global `--vm`.

## Capabilities

### New Capabilities
- `vm-management`: several VMs, the Workspace, the New VM wizard, move, export/import, running detection and the theme preference.

### Modified Capabilities
<!-- None beyond the additions above. -->

## Impact

- New dependency: `modernc.org/sqlite`.
- Settings storage changes from `config.json` to the database; `--config <file>` still uses a standalone JSON file.
- The per-VM "workspace" is now called the VM's folder in the UI and CLI (`--folder`); the stored field keeps the name `workspace`.

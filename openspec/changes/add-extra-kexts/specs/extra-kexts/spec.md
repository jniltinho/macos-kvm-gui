# Spec Delta

## Purpose

Let the user give OpenCore kexts the app does not ship, chosen and supplied by the user in the settings file, so GPUs that need a patch kext can be passed through; and keep the user's settings in one YAML file.

## ADDED Requirements

### Requirement: Settings file
The app SHALL keep user settings in `settings.yaml` in its user config folder (`~/.config/macos-kvm-gui/settings.yaml`), with the keys `theme`, `assets_dir`, `kexts` and `disable`. When the file does not exist, the first read SHALL create it from a default embedded in the app that sets nothing. An unknown key or invalid YAML SHALL be reported with the file's path.

#### Scenario: First start
- **WHEN** the app reads its settings and `settings.yaml` does not exist
- **THEN** the file is created with the commented default, and behavior is the same as with no settings

#### Scenario: Unknown key
- **WHEN** `settings.yaml` has a key the app does not know
- **THEN** reading the kexts fails with an error naming the file and the key

### Requirement: Settings precedence
For `theme` and `assets_dir` the precedence SHALL be: environment variable (`MKG_THEME`, `MKG_ASSETS_DIR`), then `settings.yaml`, then the older `.env` file.

#### Scenario: Precedence
- **WHEN** `.env` has `MKG_THEME=dark`, `settings.yaml` has `theme: light` and the environment has `MKG_THEME=system`
- **THEN** the theme is `system`; without the environment variable it is `light`; with neither it is `dark`

### Requirement: Reset to default
The user SHALL be able to overwrite `settings.yaml` with the embedded default from the GUI (Preferences, after a confirmation) and from the CLI (`settings reset --yes`).

#### Scenario: CLI reset
- **WHEN** the user runs `settings reset` without `--yes`
- **THEN** nothing is written and the error asks for `--yes`

#### Scenario: Reset
- **WHEN** the user confirms a reset
- **THEN** the file holds exactly the embedded default

### Requirement: Extra kexts list
Each `kexts` item SHALL have an `https://` `url` of a zip archive, an optional `sha256` (64 hex digits) and an optional `load` list of kext names in load order. The app SHALL NOT ship, name or default to any specific third-party kext.

#### Scenario: Empty list
- **WHEN** `kexts` and `disable` are empty or absent
- **THEN** boots, downloads and GPU eligibility behave exactly as before this change

#### Scenario: Malformed item
- **WHEN** an item's `url` is not `https://`, its `sha256` is not 64 hex digits, or a `load` name is invalid
- **THEN** the VM does not start and the error names the file and the item

### Requirement: One settings snapshot per start
A VM start SHALL read the kexts once and use that result for the download, the GPU check, the copy into the EFI folder and the rendered config. Creating a VM or a workspace SHALL NOT need the extra kexts nor network access.

#### Scenario: File edited during a start
- **WHEN** `settings.yaml` changes while a start is downloading
- **THEN** that start still copies and references only the items it read first

#### Scenario: VM creation with an uncached item
- **WHEN** a VM is created while `kexts` lists an item that is not cached
- **THEN** the VM is created without downloading, and the first start downloads the item

### Requirement: Download and cache
Before a VM starts from the GUI, the CLI (`vm start`) or the MCP server, the app SHALL download every listed archive that is not cached yet into a cache shared by the Workspace's VMs, keyed by the full SHA-256 of the item (`url`, `sha256`, `load`). A cache hit SHALL count only when the stored item equals the requested one. A failure SHALL leave nothing cached for that item and stop the start with an error naming its URL.

#### Scenario: First start with a new item
- **WHEN** a VM starts and an item is not cached
- **THEN** the archive is downloaded once, and later starts reuse the cache without network access

#### Scenario: Hash mismatch
- **WHEN** the downloaded archive does not match the item's `sha256`
- **THEN** the VM does not start, the error shows the expected and actual hashes, and nothing is cached

#### Scenario: Changed item
- **WHEN** the user changes an item's `url`, `sha256` or `load`
- **THEN** the new item is downloaded, and the old item's kexts are no longer loaded

#### Scenario: Slow download from the MCP server
- **WHEN** an MCP client starts a VM whose item takes longer than the server's start timeout to download
- **THEN** the download finishes before the VM process is launched, and the timeout applies only to the VM showing up

### Requirement: Concurrent downloads
Concurrent starts SHALL download an item at most once and SHALL NOT remove an item another start published.

#### Scenario: Two starts at once
- **WHEN** two VMs start at the same time with the same uncached item
- **THEN** one downloads it, the other waits and reuses it, and both start

### Requirement: Archive validation
Before extracting anything, the app SHALL reject an archive that is larger than 32 MiB, expands to more than 128 MiB, has more than 4096 entries, has any entry with an absolute path, a `..` element or a symbolic link, or any file larger than 16 MiB. A size limit SHALL fail the download, never truncate a file.

#### Scenario: Unsafe archive
- **WHEN** an archive holds a valid `Foo.kext` plus a symbolic link or a `../x` entry anywhere
- **THEN** the download fails and nothing is extracted

#### Scenario: Oversized file
- **WHEN** a file in the archive is larger than the limit
- **THEN** the download fails with a size error and nothing is cached

### Requirement: Kext layout
Each kext SHALL be a root-level `<Name>.kext` folder with a regular `Contents/Info.plist` and a regular `Contents/MacOS/<Name>`, and `<Name>` SHALL match `[A-Za-z0-9._+-]{1,64}`. An archive with one kext needs no `load`; an archive with several SHALL be refused unless `load` lists which ones to load, in load order.

#### Scenario: Missing Info.plist
- **WHEN** a kext has `Contents/MacOS/<Name>` but no regular `Contents/Info.plist`
- **THEN** the download fails naming the kext

#### Scenario: Several kexts without a list
- **WHEN** an archive holds `A.kext` and `B.kext` and the item has no `load`
- **THEN** the download fails and the error lists `A` and `B`

#### Scenario: Several kexts with a list
- **WHEN** the item has `load: [B, A]`
- **THEN** only `B` and `A` are loaded, `B` before `A`

### Requirement: Boot injection
Every start SHALL copy the cached kexts of the snapshot into OpenCore's kext folder and add one enabled `Kernel > Add` entry per kext, in this order: the built-in kexts, root-patch mode's kexts, then the extra kexts in list order and, within an item, in archive or `load` order.

#### Scenario: Kext loaded
- **WHEN** a VM starts with one cached item holding `Foo.kext`
- **THEN** OpenCore's config has an enabled entry for `Foo.kext` after every built-in and root-patch entry, and the EFI folder has `Foo.kext`

#### Scenario: Item removed
- **WHEN** the user removes the item and starts again
- **THEN** no extra kext is in the EFI folder or the config

### Requirement: Boot injection errors
A kext name that equals another extra kext's name, or a built-in or root-patch kext's name, ignoring case, SHALL stop the start with an error naming both. A start whose snapshot has an item that is not cached SHALL fail with an error telling the user to start the VM from the GUI, `vm start` or the MCP server, which download it.

#### Scenario: Name clash ignoring case
- **WHEN** an item provides `Whatevergreen.kext`
- **THEN** the VM does not start and the error names it and `WhateverGreen.kext`

### Requirement: Disabling built-in kexts
The `disable` list SHALL name built-in kexts (without `.kext`) whose `Kernel > Add` entries are disabled. Lilu SHALL NOT be disableable. A name that is unknown, or that the config template in use offers no switch for, SHALL stop the start with an error naming it.

#### Scenario: WhateverGreen disabled
- **WHEN** `disable: [WhateverGreen]`
- **THEN** the WhateverGreen entry is disabled and every other built-in kext stays enabled

#### Scenario: Lilu
- **WHEN** `disable` lists `Lilu`
- **THEN** the VM does not start and the error says Lilu is required

#### Scenario: Override template without the switch
- **WHEN** `assets_dir` points to a template without the switch for WhateverGreen and `disable` lists it
- **THEN** the VM does not start and the error says the template cannot disable WhateverGreen

### Requirement: Patch-dependent GPU eligibility
AMD GPUs that macOS drives only with a patch kext (`699f`, `6fdf`, `73af`, `73a5`, `73df`, `73ef`) SHALL be eligible for passthrough, subject to every other eligibility check, while `kexts` has at least one valid item. Otherwise their reason SHALL say they need a patch kext listed in `settings.yaml`, or show the file's error. A start SHALL apply the same rule to its own snapshot. The app SHALL NOT check which kext was supplied.

#### Scenario: RX 6700 XT with a kext listed
- **WHEN** a `1002:73df` GPU passes the IOMMU, display and group checks and `kexts` has a valid item
- **THEN** the GPU tab lists it as eligible

#### Scenario: RX 6700 XT without kexts
- **WHEN** `kexts` is empty
- **THEN** the GPU is not eligible and the reason names `settings.yaml`

#### Scenario: Kexts removed after choosing the GPU
- **WHEN** a VM has a `73df` GPU and starts with no kexts in its snapshot
- **THEN** the VM does not start and the error names `settings.yaml`

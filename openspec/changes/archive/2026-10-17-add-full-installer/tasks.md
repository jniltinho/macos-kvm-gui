# Tasks

## 1. Catalog and download
- [x] 1.1 Fetch and parse Apple's Software Update catalog; find InstallAssistant.pkg products, their version/build and URLs
- [x] 1.2 Pick the newest build of the VM's macOS major version; refuse High Sierra–Catalina with a clear message
- [x] 1.3 Resumable download with progress into the shared cache; free-space check first
- [x] 1.4 Verify against Apple's signed integrity data (chunklist); refuse and delete on mismatch

## 2. Delivery
- [x] 2.1 Installer disc: ISO 9660 with the package in pieces under 4 GiB (macOS's cd9660 reads no multi-extent file) and the install script; attached with install media, snapshot=on
- [x] 2.2 Install step 4: the line for the recovery's Terminal, with a copy button
- [x] 2.3 Type it in the VM (Utilities › Terminal through the menu bar with Control+F2, then the line in layout-independent keys) and Unattended (-u: erase the empty disk, startosinstall --agreetolicense, reboot)

## 3. GUI, CLI, docs, tests
- [x] 3.1 Install step 2: "Download the full installer" next to the recovery, cache status and delete
- [x] 3.2 CLI `download --full`, `installer list|remove`
- [x] 3.3 Tests: catalog parsing, version pick, integrity check, disc (read back and pieces joined), attach, cache
- [x] 3.4 End-to-end test on the reference host: Sequoia 15.8.1 (guided and unattended) and Big Sur 11.7.11 (unattended, through the MCP server) installed from the disc to Setup Assistant; Tahoe still to test — Tahoe waived by the user on 2026-10-17; Sequoia and Big Sur done as recorded.
- [x] 3.5 Docs: getting started, troubleshooting, architecture, CLI

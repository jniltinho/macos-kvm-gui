# Proposal

## Why

Today the app downloads only the macOS recovery (`BaseSystem.dmg`, a few GB). **Reinstall macOS** inside the VM then downloads the full system (13–15 GB) from Apple again for every VM and every attempt, through the VM's NAT network, without progress outside the guest and without resuming. Labs that install several VMs of one version, or retry an install, pay that download each time.

## What Changes

- **Download the full installer** (optional, Install step 2): the app reads Apple's public Software Update catalog over HTTPS, picks the newest public `InstallAssistant.pkg` of the VM's macOS version (Big Sur 11 and later) whose distribution lists the VM's Mac board ID (which leaves out Apple-silicon-only packages), downloads it with progress and HTTP Range resume, and checks its size and every chunk's SHA-256 against Apple's integrity data before using it. That integrity data (CNKL, checksum method 2) is not signed with Apple's key like the recovery's chunklist; its authenticity comes from Apple's HTTPS servers, and redirects away from HTTPS are refused.
- **Shared cache:** installers live once per macOS build in `<Workspace>/.installers/<build>/InstallAssistant.pkg`, reused by every VM of that version; the UI shows the cached version and size and can delete it.
- **Delivery to the VM, offline:** the verified package is written into an ISO 9660 disc in the cache, as pieces of under 4 GiB (macOS's cd9660 reads no multi-extent file, so one 15 GB file shows as several 4 GiB ones), with an install script; the package is then deleted. A VM that starts with install media gets the disc as a disk, its writes discarded. Install step 4 shows one line to type in the recovery's Terminal, `sh /Volumes/OFFLINE/INSTALL.SH`: it copies the recovery's own installer app onto the erased disk, joins the pieces into its `SharedSupport.dmg` and opens it (the OSX-KVM offline method). An earlier HTTP server on the host's loopback, reached through NAT, needed `installer -pkg` and a `pkgutil` fallback and the network; it was replaced. FAT32, vvfat, exFAT and HFS+ images were ruled out (4 GB or 2 GB limits, FUSE or root).
- **CLI:** `download --full` and `installer list|remove`.
- **Out of scope:** High Sierra to Catalina (other package layout), creating a bootable USB, building an APFS/HFS+ image on Linux, and automating the guest's Terminal.

## Capabilities

### New Capabilities
- `full-installer`: download, verification, caching and delivery of Apple's full macOS installer.

## Impact

- New core code for the catalog, the integrity data and the local file server; no new dependency.
- The recovery download stays as it is and remains the default; the full installer is an addition.

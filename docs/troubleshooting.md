# Troubleshooting

[Documentation index](README.md)

Start with **Host › Check again** and the **Log** tab (last 1000 session lines). Quoted messages below are literal core messages; `<...>` denotes a substituted value. Install dialogs can prepend an operation name, and the Log can prepend `error:` or `[VM]`.

## KVM, workspace and disk access

| Message | Action |
| --- | --- |
| `KVM is busy: another hypervisor (VirtualBox or VMware) is using virtualization. Shut its VM down and try again.` | Shut down the other hypervisor's VM, not merely its manager window. Retry. The app does not kill it or unload modules. This translation follows QEMU's `failed to initialize kvm: Device or resource busy`. |
| `The disk is already open in another QEMU (started outside the app?). Shut that VM down first.` | Shut down the external QEMU using that disk. Do not disable disk locking or delete files to bypass it. This translation follows `Failed to get "write" lock`. |
| `workspace in use by another operation or another app instance` | Finish the other app operation or stop its VM. Kernel-held `flock` locks release on process death; deleting a lock file is not the fix. |
| `No access to KVM. Check the Host tab.` | Use **Set up host**, then fully log out/in if group membership changed. Manual equivalent for permissions: `sudo usermod -aG kvm "$USER"`. If the node `does not exist`, enable virtualization and load `kvm_intel` or `kvm_amd` for the host CPU. |
| `VT-x/AMD-V missing` | Enable hardware virtualization in BIOS/UEFI; nested environments must expose it too. |
| `Ventura or later requires SSE4.1 and AVX2` | Select a version through Monterey or use compatible hardware. Older versions remain experimental. |
| `mac_hdd_ng.img does not exist: create the disk in the Install tab` | Check the saved workspace path and create its disk. Do not replace a disk containing an existing installation. |
| `VM folder <path> does not exist: create it in the Install tab` | Check the VM's folder in Settings › General and use Create VM folder in Install. |

The Host tab's QEMU-version hint is `QEMU 8.2.2 or later is required`; missing tools show `not found in PATH`. Install/upgrade `qemu-system-x86` and `qemu-utils`, then recheck.

## ignore_msrs

Host status `disabled: macOS will kernel panic` means **Set up host** is needed. Alongside QEMU/module/group setup, it writes `1` to `/sys/module/kvm/parameters/ignore_msrs` and this persistent configuration:

```text
# /etc/modprobe.d/macos-kvm-gui.conf
options kvm ignore_msrs=1 report_ignored_msrs=0
```

Use the app's `pkexec` password prompt; do not launch the whole GUI as root. Check the actual value with:

```sh
cat /sys/module/kvm/parameters/ignore_msrs
```

`Y` or `1` is enabled. Current host-setup messages include:

- `the CPU reports no VT-x/AMD-V: enable virtualization in the BIOS/UEFI first` — software setup cannot enable firmware virtualization.
- `host setup failed: <error>` — inspect the appended system error. If it says `apt-get not found: install <packages> with your package manager`, install the missing QEMU packages with your distribution's tools. Setup does not run `apt-get update` or upgrade existing QEMU binaries.
- `host setup ran, but ignore_msrs is still disabled` — inspect the running KVM parameter and permissions, then retry. Earlier package/group changes may already have happened; setup is not transactional.
- `<user> was added to the kvm group: log out and back in (or reboot) so /dev/kvm opens.` — start a fresh login session, relaunch and recheck.
- `The host is ready for KVM.` — setup's KVM access/MSR checks passed; still review the other Host checks. Cancelling authorization shows `Cancelled: nothing was changed.`

Host status `kvm module not loaded` is inconclusive, not proof that the setting is disabled. If KVM loads from an initramfs, regenerate it after the persistent fix (`sudo update-initramfs -u` on Debian/Ubuntu). The Host button does not do that rebuild.

## GPU start guards and reservation

See [GPU passthrough](gpu-passthrough.md) for eligibility and broken-reservation recovery. The feature is **untested on real hardware**.

- `macOS drives this GPU only with a patch kext: list one in …/settings.yaml` — a patch-dependent AMD GPU (for example an RX 6700 XT) needs a kext you supply; see [patch-dependent AMD GPUs](gpu-passthrough.md#patch-dependent-amd-gpus).

| Message template | Action |
| --- | --- |
| `GPU <address> not found on this host` | Revisit GPU selection after changing hardware/PCI slots, or select None. |
| `GPU <address> cannot be passed through: <reason>` | Follow the specific eligibility reason; do not bypass the boot-display, IOMMU or duplicate-ID safeguards. |
| `this GPU needs macOS <version> or newer` | Correct the selected macOS family and use a guest new enough for the card. Point-version requirements are not checked automatically. |
| `<address> is bound to "<driver>", not vfio-pci: reserve the GPU in the GPU tab and reboot the host` | Reserve and reboot. Selecting a card alone does not rebind it. |
| `cannot open /dev/vfio/<group> (<error>): check the udev rule from the reservation and that you are in the kvm group (log in again after joining it)` | Confirm the reservation's udev rule, group-node permissions and current-session `kvm` membership. Fully log out/in after adding the group. Do not work around it with root QEMU or world-writable device nodes. |
| `the soft locked-memory limit is <current> MiB but the VM needs <required> MiB (VFIO pins all guest RAM): raise memlock in /etc/security/limits.d and log in again` | Raise both soft/hard limits for your user or reduce guest RAM within supported limits. Relaunch the app from a session with the new limit. |

For an **8192 MiB** guest, a PAM limits file can use the following (replace `your-user`; memlock values here are **KiB**, unlike the app's MiB):

```text
your-user soft memlock 8388608
your-user hard memlock 8388608
```

Use higher values for larger guests. Log in again and check `ulimit -l` in the launching session. A GUI launched through a user service may inherit service limits instead; verify those rather than assuming a shell change affects the app.

Reservation failures are prefixed `reservation failed: <error>`; release failures use `release failed: <error>`. Important script messages:

- `another GPU change is running` — wait for the other host-wide reserve/release operation.
- `initramfs rebuild failed; the reservation was rolled back` — fix the rebuild cause and verify a successful rebuild before rebooting.
- `files removed, but the initramfs rebuild failed: run sudo update-initramfs -u -k all` — the old initramfs may still reserve the GPU; run the indicated command successfully before rebooting.

## Recovery download, verification and conversion

| Message / template | Action |
| --- | --- |
| `download failed: recovery: session: <error>` | Check connectivity/proxy access to Apple's recovery service, then retry. |
| `download failed: recovery: no session cookie in server response` | A proxy/captive portal or changed server response may be interfering. Restore direct valid access and retry. |
| `download failed: recovery: server response missing key <key>` | Retry later and check Apple's service/proxy response; do not substitute an unverified image. |
| `download failed: recovery: download <file>: unexpected status <status>` | Check network/server access and retry; a fresh download obtains new tokens. |
| `not enough free space: 4 GiB needed and <available> GiB available` | Free workspace space. Four GiB is only the initial check, not the total storage needed. |
| `verification failed: invalid chunklist signature` | Do not bypass verification. Retry on a trusted connection; investigate persistent failures. |
| `verification failed: chunklist has no Apple digital signature` | Reject the image/chunklist pair; download again. |
| `verification failed: chunk <n> does not match (corrupted image)` | Retry the download and check storage/network integrity. |
| `verification failed: image truncated at chunk <n>` or `verification failed: image larger than the chunklist` | The DMG does not match the signed chunklist. Retry; never convert/use the failed image. |

Malformed lists may also report `verification failed: chunklist has an invalid header`, `verification failed: chunklist has an invalid size` or `verification failed: chunklist truncated: <error>`.

Conversion reports:

```text
conversion failed (if qemu-img does not support this compression, install dmg2img): <error>
```

Read the last `qemu-img` line for unsupported compression versus disk-space/permission failure. Upgrade/fix `qemu-img` as appropriate. **The app does not invoke `dmg2img` or fall back automatically**; installing it alone does not change the app's conversion command. A manual alternative must still use a verified DMG. Failed download/verification/conversion leaves previous `BaseSystem.img` intact and removes staging on return.

`warning: could not record the media version: <error>` means publication succeeded but the marker could not be written. The image remains usable; fix workspace write access. An imported image without a marker is also shown as version `unknown`.

## Disk space, QMP and boot files

- `mac_hdd_ng.img already exists; nothing was changed` — intentional overwrite protection. Use the existing disk or a different workspace.
- `the disk must be at least 40 GiB` — increase the requested virtual size.
- `the host has only <free> GiB free: the disk is thin and the VM may stop when space runs out` — creation succeeded but storage is not reserved. Free space before filling the guest disk.
- `I/O error on disk <device> (<operation>): out of space on the host` — QEMU paused the guest. Free host space and preserve data; there is no resume button in this app. Avoid Force stop unless necessary.
- `QMP control unavailable: use Force stop` or `warning: QMP connection closed; graceful shutdown is unavailable` — wait briefly if just started, or shut down inside macOS. Force stop is destructive; the app never escalates automatically.
- `could not prepare the boot files: MKG_ASSETS_DIR=<directory> is missing <file>` — fix the complete override tree or unset the override/use built-ins. The setting may come from `assets_dir` in `settings.yaml` or the older `.env`.
- `…/settings.yaml: …` (a YAML error, `field … not found`, or `only one YAML document is allowed`) — fix the file, or **File › Preferences › Reset to default** / `settings reset --yes`. Until then VMs do not start and patch-dependent GPUs are not eligible.
- `<url>: the download's SHA-256 is <a>, the entry expects <b>` — the archive changed or the hash is wrong; check it and update `sha256`. Nothing was cached.
- `<url>: the archive has several kexts (A, B): choose them, in load order, with load: [A, B]` — add `load` to the item.
- `<url>: … larger than … MiB`, `unexpected path`, `unexpected link`, `has no Contents/Info.plist`, `no root-level <Name>.kext…` — the zip is not a plain kext release archive; the app refuses it before extracting anything.
- `the settings.yaml entry <url> is not downloaded` — the start was prepared without downloading (for example `workspace create` followed by a manual boot); start the VM from the app, `vm start` or the MCP server.
- `<Name>.kext from settings.yaml clashes with WhateverGreen.kext` — the kext has the name of a built-in, root-patch or other extra kext (case is ignored). To replace a built-in one, use `disable` and a kext with its own name.
- `settings.yaml disable: <Name> is not a built-in kext the config template in use can disable` — only `VirtualSMC` and `WhateverGreen` can be disabled, and an `assets_dir` template needs the `@@ENABLED_<Name>@@` switches.

## Stuck at the Apple logo

If the Apple logo stays without a progress bar, use **Start with verbose boot** in Install step 4, or **Machine › Start with verbose boot (debug)** (CLI: `vm start --verbose-boot`). That start, and only that one, adds `-v debug=0x100` to the boot arguments: macOS prints its boot log on the screen instead of the logo, and stops on a panic with its details.

- **The log ends at `EB|LOG:EXITBS:START`** and the VM uses one CPU at 100% without reading the disk: the kernel never started. The usual cause is a core count that is not a power of two, such as 6 cores × 2 threads. Use one of the combinations below; the app refuses other core counts.
- **A panic message** names the kernel extension or the reason; search it together with the macOS version.
- **The log keeps scrolling:** macOS is still starting. The first boot of a recovery or an installer can take several minutes without GPU acceleration.

### Recovery language

The recovery and the installer start in **English with the host's keyboard layout**: OpenCore sets the `prev-lang:kbd` NVRAM variable at every boot from the host's layout (`/etc/default/keyboard`, else `localectl`): `en-US:0` for U.S., `en-US:128` for Brazilian ABNT2, and so on; unknown layouts get U.S. Keys reach macOS by physical position. On ABNT2, the extra `/?` key right of `;` does not reach macOS (QEMU's USB keyboard sends it as a Japanese key); use the keypad `/`. (Releases up to 0.7.0 kept the value from OpenCore's sample configuration, `ru-RU:252`, so the recovery opened in Russian.) To use another language for the installer, pick it in the recovery's menu; the installed macOS uses the language chosen in Setup Assistant.

### Valid processor counts

In Settings › System › Processor, **Processors** is the total number of vCPUs and **Threads per core** is 1 or 2, so the cores are processors ÷ threads. The cores must be 1, 2, 4, 8 or 16:

| Cores | 1 thread per core | 2 threads per core |
| --- | --- | --- |
| 1 | 1 vCPU | 2 vCPUs |
| 2 | 2 vCPUs | 4 vCPUs |
| 4 | 4 vCPUs | 8 vCPUs |
| 8 | 8 vCPUs | 16 vCPUs |
| 16 | 16 vCPUs | 32 vCPUs |

Any other total cannot be split into such cores: with 2 threads per core, 6, 10, 12, 14, 20 and 24 vCPUs give 3, 5, 6, 7, 10 and 12 cores, and macOS hangs at the Apple logo. 12 vCPUs, for example, must become 8 or 16.

| Tried on the reference host (Ryzen 9 9900X, 24 threads, QEMU 8.2.2) | Result |
| --- | --- |
| 2 cores × 2 threads = 4 vCPUs, 8 GB (Sequoia recovery) | Boots to macOS Recovery |
| 4 cores × 2 threads = 8 vCPUs, 16 GB (Sequoia recovery) | Boots: the kernel starts and launchd runs |
| 8 cores × 2 threads = 16 vCPUs, 16 GB (Sonoma, installed) | Boots and runs (the validated VM) |
| 6 cores × 2 threads = 12 vCPUs, 16 GB (Sequoia recovery) | Hangs at the Apple logo after `EXITBS:START` |

Other combinations in the table follow the same rule but were not booted one by one. Do not give the VM more vCPUs than the host has threads, and leave some for the host: **Recommended for this host** proposes about half of them as a power-of-two core count (24 host threads give 4 cores × 2 = 8 vCPUs; 32 or more give 8 × 2 = 16). Without a GPU the CPU draws the screen, so more vCPUs make it smoother, up to what the host can spare.

## Full installer

**Download full installer** (Install step 2, or `download --full`) fetches Apple's `InstallAssistant.pkg` for the VM's macOS version from the public Software Update catalog. Only Big Sur and later have one; older versions install from the recovery. The app picks the newest public release that supports the VM's Mac model (so Apple-silicon-only versions are skipped), checks free space, resumes an interrupted download and checks every chunk's SHA-256 against Apple's integrity data. That data is not signed with Apple's key like the recovery's chunklist; it and the package come from Apple's HTTPS servers, and the app refuses redirects away from HTTPS.

After the check, the app writes the package into an installer disc, `<Workspace>/.installers/<build>/InstallAssistant.iso` (ISO 9660, volume `OFFLINE`), and deletes the package; for that moment the download needs twice its size free. The package is on the disc as pieces `PKG.000`, `PKG.001`, … of under 4 GiB, because macOS reads no ISO 9660 file over 4 GiB.

While a VM starts with install media, the disc is attached to it as a disk (its writes are discarded, so the cache stays unchanged). No network is needed. Install step 4 shows the line to type in the recovery's Terminal, `sh /Volumes/OFFLINE/INSTALL.SH`. What it does, in order:

1. Copies the recovery's own "Install macOS ….app" to "/Volumes/Macintosh HD/private/tmp" (the recovery's volume is read-only).
2. Joins the pieces into the app's `Contents/SharedSupport/SharedSupport.dmg`, which takes a few minutes. The disk needs room for the package and macOS itself: use a disk of 80 GB or more.
3. Opens the installer. Continue there, choosing "Macintosh HD".

**Type it in the VM** does the typing: once the recovery shows its window (not earlier: keys pressed while macOS boots change how it boots), it opens Utilities › Terminal through the menu bar (Control+F2, then the arrow keys: Big Sur's recovery has no Terminal shortcut) and types `sh /V*/OFFLINE/I*`, the same line spelled with keys that give the same characters in every Latin QWERTY, AZERTY or QWERTZ layout (no A, M, Q, W, Y or Z, and the keypad's `/`, `*` and `-`). Caps Lock must be off. With Dvorak or Russian, it refuses: type the line yourself.

**Unattended** (the check box, or `-u` after the line) needs no Disk Utility and no clicks: the script erases the VM's empty disk as APFS "Macintosh HD" (the only internal disk with no partitions that is not mounted; with none or several it stops), then runs the installer's `startosinstall --agreetolicense`, which **accepts Apple's macOS license for you**, and restarts the VM. The installation continues on its own (about 30 minutes) up to Setup Assistant.

- **"No volume at /Volumes/Macintosh HD":** erase the disk in Disk Utility as APFS named "Macintosh HD", or give the volume: `sh /Volumes/OFFLINE/INSTALL.SH "/Volumes/Your Disk"`.
- **No `/Volumes/OFFLINE`:** the disc is attached only when the VM starts with install media and a cached installer of its macOS version; shut the VM down and start it again after the download.
- **Delete installer** (Install step 2, or **File › Offline installers**, which lists every cached one and the VMs that use it) refuses while a VM is using it.

## Network adapters

Each VM has up to four adapters (Settings › Network, Adapter 1–4), attached to **NAT** or to a **Bridged Adapter**, with a type and a MAC address.

| Adapter type | macOS with a built-in driver |
| --- | --- |
| Paravirtualized Network (`virtio-net-pci`) | Big Sur 11 and later |
| VMware `vmxnet3` | High Sierra 10.13 through Tahoe 26 |
| Intel PRO/1000 MT Desktop (`e1000-82545em`) | High Sierra through Monterey 12 (Apple removed the driver in Ventura) |

New adapters default to `vmxnet3` through Catalina and `virtio-net-pci` from Big Sur. Settings warns when the chosen type has no driver for the VM's macOS version.

- **No network in macOS:** check the adapter type against the table above.
- **"SSH port … is in use":** another VM (or program) already listens on that port on `127.0.0.1`. Every VM defaults to 2222; give each VM its own port in Settings › Network.
- **The MAC address.** VMs created before adapters became configurable keep the address they always had (`52:54:00:c9:18:27`), so macOS keeps seeing the same interface. New VMs and new adapters get a random, locally administered address. Changing the first adapter's MAC or type can make macOS treat it as a new interface; iCloud and Messages may then ask you to sign in again. Two VMs on the same bridge must not share a MAC: regenerate it in a copied VM.
- **Several NAT adapters** each get their own private subnet (10.0.2.0/24, 10.0.3.0/24, …). The SSH forward is only on the first enabled NAT adapter.

### Bridged networking

A bridged adapter puts the VM on a host bridge such as `br0`, through QEMU's `qemu-bridge-helper`. The app and QEMU stay unprivileged, but the helper needs privileges, and the app never changes the host's network configuration. One-time setup, as an administrator:

1. **Create a bridge** with your network tools (NetworkManager, netplan or systemd-networkd), enslaving a wired interface. Wi-Fi interfaces usually cannot be bridged. `virbr0` (libvirt) and Docker bridges exist but are NAT or isolated networks, not your LAN.
2. **Allow the bridge** for the helper, in `/etc/qemu/bridge.conf` (create the file if it is missing):

   ```text
   allow br0
   ```

   `allow all` also works, and a `deny` line wins over any `allow`.
3. **Make the helper setuid root.** Fedora ships it that way; Ubuntu and Debian do not. Use `dpkg-statoverride` so package upgrades keep the change:

   ```sh
   sudo dpkg-statoverride --update --add root root 4755 /usr/lib/qemu/qemu-bridge-helper
   ```

The helper is `/usr/lib/qemu/qemu-bridge-helper` on Ubuntu/Debian and `/usr/libexec/qemu-bridge-helper` on Fedora. Before starting, the app checks that the bridge exists, the helper is installed and, when it can read `bridge.conf`, that the bridge is allowed. Anything else (helper privileges, `/dev/net/tun`) shows up as QEMU's error: "A bridged adapter could not attach…". A bridged VM gets its address from the bridge's network (DHCP on your LAN, for example); there is no `127.0.0.1` SSH forward on that adapter, so use the VM's address with **Remote Login** enabled in macOS.

## Screen, keyboard and pointer

- **Shut down does nothing.** The request reaches macOS (the Log tab shows "ACPI power button sent to macOS"), but macOS treats the power button as Sleep (`pmset -g` shows `Sleep On Power Button 1`) and a VM cannot sleep. Use **Apple menu › Shut Down** inside macOS, `sudo shutdown -h now` over SSH, or **Force stop** if macOS is frozen.
- **Two pointers.** Fixed: the host pointer is hidden over the screen, and only macOS's pointer, drawn into the framebuffer, remains. If you still see two, use the latest build.
- **Pointer lag.** Without GPU acceleration macOS draws its pointer on the CPU, and QEMU's VNC server lowers its update rate when the screen is idle, so the pointer can trail the hand slightly. The app redraws at up to 60 fps. Give the VM more vCPUs (Settings › Recommended), or use **Display: QEMU window** for comparison.
- **Wrong characters.** Keys are sent by physical position with a US layout. Keep macOS on the **ABC** or **U.S.** input source. Keys that only exist on other layouts (for example the extra key next to the left Shift on ISO/ABNT keyboards) may not reach macOS.
- **Stuck in full screen.** Press **Ctrl+Alt+F**.
- **Apple ID window after switching from OSX-KVM.** The app's SMBIOS (iMac20,1 with a new serial) makes macOS see a new Mac and ask you to sign in again. Close the window or sign in.

## Display and graphics

These are limitations, not translated core error messages:

- **No Metal / slow desktop:** macOS uses the boot framebuffer of the emulated `vmware-svga` device, without graphics acceleration. Switching to QXL/virtio-gpu or increasing VRAM does not supply a macOS Metal driver. QEMU's `apple-gfx` is for macOS hosts, not this Linux/KVM path.
- **Blank Sonoma wallpaper:** choose a static picture (for example `/System/Library/Desktop Pictures/Sonoma.heic`) instead of the animated default. Enable **Reduce motion** and **Reduce transparency**, and disable Dock animation.
- **wgpu/Metal-based applications fail:** a graphics API/backend that requires Metal cannot use the unaccelerated framebuffer. Use a software/non-Metal backend only if that application offers one; more vCPUs or VRAM does not add Metal. Physical supported AMD passthrough is the app's acceleration route, but is untested.
- **1280×800 instead of 1920×1080:** this app generates OpenCore `Resolution=1920x1080@32`; the older OSX-KVM OpenCore image described in the repository leaves it empty and can boot at 1280×800. Start through this app and check that built-in (or matching override) assets are used. There is no resolution selector. Do not delete persistent NVRAM merely to change resolution; extracted EFI edits are replaced on Start.
- **OpenCore keeps showing recovery:** keep choosing macOS Installer/installed disk during installation. Once the installed system boots, stop the VM and remove media. Media presence, not an installation-complete detector, controls picker visibility.

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
| `workspace <path> does not exist: prepare it in the Install tab` | Save the intended path and use Create workspace. |

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
- `could not prepare the boot files: MKG_ASSETS_DIR=<directory> is missing <file>` — fix the complete override tree or unset the override/use built-ins. The setting may come from the user-config `.env`.

## Display and graphics

These are limitations, not translated core error messages:

- **No Metal / slow desktop:** macOS uses the boot framebuffer of the emulated `vmware-svga` device, without graphics acceleration. Switching to QXL/virtio-gpu or increasing VRAM does not supply a macOS Metal driver. QEMU's `apple-gfx` is for macOS hosts, not this Linux/KVM path.
- **Blank Sonoma wallpaper:** choose a static picture (for example `/System/Library/Desktop Pictures/Sonoma.heic`) instead of the animated default. Enable **Reduce motion** and **Reduce transparency**, and disable Dock animation.
- **wgpu/Metal-based applications fail:** a graphics API/backend that requires Metal cannot use the unaccelerated framebuffer. Use a software/non-Metal backend only if that application offers one; more vCPUs or VRAM does not add Metal. Physical supported AMD passthrough is the app's acceleration route, but is untested.
- **1280×800 instead of 1920×1080:** this app generates OpenCore `Resolution=1920x1080@32`; the older OSX-KVM OpenCore image described in the repository leaves it empty and can boot at 1280×800. Start through this app and check that built-in (or matching override) assets are used. There is no resolution selector. Do not delete persistent NVRAM merely to change resolution; extracted EFI edits are replaced on Start.
- **OpenCore keeps showing recovery:** keep choosing macOS Installer/installed disk during installation. Once the installed system boots, stop the VM and remove media. Media presence, not an installation-complete detector, controls picker visibility.

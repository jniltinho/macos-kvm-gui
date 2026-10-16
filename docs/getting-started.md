# Getting started

[Documentation index](README.md)

**For home lab use.** macOS is Apple's software; its license restricts use to Apple-branded hardware and places conditions on virtualization. Review [the license for your macOS version](https://www.apple.com/legal/sla/) before proceeding. This project is not affiliated with Apple.

## Requirements

- Linux x86_64 with VT-x/AMD-V enabled in BIOS/UEFI and read/write access to `/dev/kvm`.
- QEMU **8.2.2 or newer**, `qemu-img` and `pkexec`. For Debian/Ubuntu:

  ```sh
  sudo apt install qemu-system-x86 qemu-utils pkexec
  ```

- A graphical desktop and the OpenGL/window-system libraries needed by Fyne. Release packages require **glibc 2.38+** (Ubuntu 24.04+, Debian 13+); the `.deb` declares its runtime dependencies.
- At least **4096 MiB guest RAM**; the default is 8192 MiB. Leave resources for the host. Ventura and later require host SSE4.1 and AVX2.
- A writable workspace with room for recovery and macOS. Recovery preparation checks for at least **4 GiB free**, but downloads, conversion and the installed system need additional space. The virtual disk minimum is **40 GiB**, with **80 GiB** prefilled in the UI; it grows on the host as used.
- Internet access for Apple's recovery download and the installer inside macOS.

No OSX-KVM clone, Git or Python is needed at runtime. OpenCore, its kexts and OVMF are built in. Only **Sonoma** has been validated end to end; High Sierra, Mojave, Catalina, Big Sur, Monterey, Ventura, Sequoia and Tahoe are marked experimental.

## Install a release

Download a `.deb` or `.tar.gz` and its matching `.sha256` from [GitHub Releases](https://github.com/jniltinho/macos-kvm-gui/releases). Replace `X.Y.Z` below with the release version. Check the selected artifact before installing:

```sh
sha256sum -c macos-kvm-gui_X.Y.Z_amd64.deb.sha256
# Or, for the tarball:
sha256sum -c macos-kvm-gui-vX.Y.Z-linux-amd64.tar.gz.sha256
```

**Debian package:**

```sh
sudo apt install ./macos-kvm-gui_X.Y.Z_amd64.deb
macos-kvm-gui
```

The menu entry is **macOS KVM**. Do not run the app as root: only host fixes and GPU reservation request elevation through `pkexec`.

**Standalone tarball:** install the runtime dependencies above and your distribution's OpenGL/desktop runtime libraries, then:

```sh
tar -xzf macos-kvm-gui-vX.Y.Z-linux-amd64.tar.gz
cd macos-kvm-gui-vX.Y.Z-linux-amd64
./macos-kvm-gui
```

## First run through the tabs

The tabs appear as **VM, Install, Settings, Host, GPU, Log**. For setup, use the order below.

### Host

Make the checks green before starting: CPU virtualization, `/dev/kvm`, QEMU tools/version, CPU compatibility and `ignore_msrs`. Shut down running VirtualBox/VMware VMs. Click **Check again** after fixing a problem.

Click **Set up host** to authorize one operation that installs missing QEMU packages with `apt-get`, loads the appropriate KVM module if `/dev/kvm` is absent, creates/adds you to the `kvm` group, and enables/persists `ignore_msrs` in `/etc/modprobe.d/macos-kvm-gui.conf`. Log out/in if prompted so the new group applies, then recheck. The `libvirt` and `input` groups are not needed. Setup does not upgrade an already-present old QEMU or enable virtualization in BIOS.

**The password prompt.** Set up host runs through `pkexec`, so your desktop's polkit agent asks for **your user password** (the one you use with `sudo`). The app never sees or stores it. The prompt appears even if your `sudo` is configured without a password, because polkit does not read the sudoers file. The dialog shows the beginning of the command it will run (`/bin/sh -c case "$1" in …`); that is the app's fixed setup script, with your user name, the KVM module and the package list passed as separate, validated arguments. Cancelling the prompt changes nothing, and the app reports "Cancelled: nothing was changed." Showing a friendly text instead of the command would need a polkit policy installed as root, which the app does not do.

**Why not `libvirt` or `input`?** The app runs QEMU directly, so it never talks to libvirt. Keyboard and mouse reach the VM through the QEMU window, not through evdev. Membership in `input` also lets a user read every key typed on the machine, so do not add it unless another tool needs it.

If KVM loads from an initramfs, also rebuild that initramfs after the persistent setting; see [troubleshooting](troubleshooting.md#ignore_msrs).

### Settings

Choose a workspace (absolute path, or a path starting with `~/`) and save. Defaults:

| Setting | Default |
| --- | --- |
| Workspace | `~/OSX-KVM` |
| macOS version | Sonoma |
| RAM | 8192 MiB |
| Cores / total vCPUs | 4 / 8 |
| SSH host port | 2222, bound to `127.0.0.1` |

vCPUs must be a positive multiple of cores; the SSH port must be 1024–65535. **Recommended for this host** fills CPU/RAM fields but does not save them: click **Save**. Settings cannot be saved while the VM runs.

An existing OSX-KVM workspace can reuse `mac_hdd_ng.img` and `BaseSystem.img`. The app uses its own OpenCore and new SMBIOS identity, not the old boot files. Back up an existing VM first; iCloud/Messages may see a different Mac and require sign-in again.

### Install: steps 1–5

[![Installation journey](diagrams/install-flow.png)](diagrams/install-flow.svg)

1. **Workspace — Create workspace.** Creates the folder and extracts the built-in boot files. An existing directory already counts as ready; missing boot files are prepared on Start. Status shows asset versions and their source.
2. **macOS recovery — Download macOS recovery.** Downloads the chunklist and DMG from Apple, verifies the Apple signature and every chunk's SHA-256, then converts to `BaseSystem.img`. Progress covers download, verification and conversion. Temporary DMG/chunklist files are removed. A failed download, verification or conversion preserves any previous install image.
3. **Disk — Create disk.** Choose the virtual size in GiB. This creates a thin qcow2 file named `mac_hdd_ng.img`, never overwriting an existing disk. The size is not reserved on the host; monitor free space.
4. **Install inside the VM.** Open **VM**, click **Start**, then follow the manual guest steps below.
5. **Clean up — Remove install media.** Only after macOS boots from its installed disk, shut down the VM and confirm removal. This deletes `BaseSystem.img` and its version marker, not the macOS disk.

### VM: install macOS and first boot

1. With `BaseSystem.img` present, OpenCore shows its picker with a five-second timeout. Choose **macOS Base System**.
2. In recovery, open **Disk Utility**, enable **View › Show All Devices**, select the **QEMU HARDDISK** target and **Erase** it with **GUID Partition Map** and **APFS**. Erase only the intended virtual disk, not the recovery media.
3. Close Disk Utility, choose **Reinstall macOS**, and select that disk. The app does not automate partitioning or installation.
4. Allow the installer to restart several times. In OpenCore choose **macOS Installer** while installation is in progress, then the installed disk. Do not repeatedly restart recovery installation.
5. Complete macOS Setup Assistant and confirm that the installed system reaches its desktop. Shut down macOS, wait for **Stopped**, then use Install step 5.
6. Start again without media: OpenCore hides the picker and scans APFS/HFS+ volumes to boot the installed system. Hold **Esc** at boot if you need the picker. NVRAM and the workspace's SMBIOS identity persist across starts.

**Shut down (ACPI)** sends a request; the state remains Running until QEMU exits. Prefer shutdown inside macOS if needed. **Force stop** is like pulling the power cord and can lose data. The app blocks closing its window while the VM is running or paused.

### Optional tabs and next steps

- **GPU:** leave **None (emulated display)** selected for initial setup. [GPU passthrough](gpu-passthrough.md) is optional and **untested on real hardware**.
- **Log:** shows the last 1000 lines of app/QEMU output in this session. Use it with [troubleshooting](troubleshooting.md).
- Enable **Remote Login** inside macOS before connecting with `ssh -p 2222 user@127.0.0.1` (use your configured port/account).
- The emulated display has no Metal acceleration. Use a static wallpaper and Reduce motion/transparency; see [display limitations](troubleshooting.md#display-and-graphics).
- For a light interface, set `MKG_THEME=light` before launching; [settings and overrides](architecture.md#settings-and-overrides) explains the `.env` alternative.

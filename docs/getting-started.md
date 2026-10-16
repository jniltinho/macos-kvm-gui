# Getting started

[Documentation index](README.md)

**For home lab use.** macOS is Apple's software; its license restricts use to Apple-branded hardware and places conditions on virtualization. Review [the license for your macOS version](https://www.apple.com/legal/sla/) before proceeding. This project is not affiliated with Apple.

## Requirements

- Linux x86_64 with VT-x/AMD-V enabled in BIOS/UEFI and read/write access to `/dev/kvm`.
- QEMU **8.2.2 or newer**, `qemu-img` and `pkexec`. For Debian/Ubuntu:

  ```sh
  sudo apt install qemu-system-x86 qemu-utils pkexec
  ```

- A graphical desktop and the OpenGL/window-system libraries needed by Fyne. Release packages require **glibc 2.38+** (Ubuntu 24.04+, Debian 13+, Fedora 39+); the `.deb` and `.rpm` declare their runtime dependencies.
- At least **4096 MiB guest RAM**; the default is 8192 MiB. Leave resources for the host. Ventura and later require host SSE4.1 and AVX2.
- A writable workspace with room for recovery and macOS. Recovery preparation checks for at least **4 GiB free**, but downloads, conversion and the installed system need additional space. The virtual disk minimum is **40 GiB**, with **80 GiB** prefilled in the UI; it grows on the host as used.
- Internet access for Apple's recovery download and the installer inside macOS.

No OSX-KVM clone, Git or Python is needed at runtime. OpenCore, its kexts and OVMF are built in. Only **Sonoma** has been validated end to end; High Sierra, Mojave, Catalina, Big Sur, Monterey, Ventura, Sequoia and Tahoe are marked experimental.

## Install a release

Download a `.deb`, `.rpm` or `.tar.gz` and its matching `.sha256` from [GitHub Releases](https://github.com/jniltinho/macos-kvm-gui/releases). Replace `X.Y.Z` below with the release version. Check the selected artifact before installing:

```sh
sha256sum -c macos-kvm-gui_X.Y.Z_amd64.deb.sha256
# Or, for the RPM or the tarball:
sha256sum -c macos-kvm-gui-X.Y.Z-1.x86_64.rpm.sha256
sha256sum -c macos-kvm-gui-vX.Y.Z-linux-amd64.tar.gz.sha256
```

**Debian package:**

```sh
sudo apt install ./macos-kvm-gui_X.Y.Z_amd64.deb
macos-kvm-gui
```

**RPM package (Fedora 39+):**

```sh
sudo dnf install ./macos-kvm-gui-X.Y.Z-1.x86_64.rpm
macos-kvm-gui
```

It pulls in `qemu-system-x86`, `qemu-img` and `polkit`. RHEL, Rocky and AlmaLinux ship only `qemu-kvm`, without `qemu-system-x86_64`, so `dnf` refuses the package there.

The menu entry is **macOS KVM**. Do not run the app as root: only host fixes and GPU reservation request elevation through `pkexec`.

**Standalone tarball:** install the runtime dependencies above and your distribution's OpenGL/desktop runtime libraries, then:

```sh
tar -xzf macos-kvm-gui-vX.Y.Z-linux-amd64.tar.gz
cd macos-kvm-gui-vX.Y.Z-linux-amd64
./macos-kvm-gui
```

## First run through the tabs

The window works like VirtualBox's manager: the **File / Machine / Help** menus, a toolbar (**New**, **Import** | **Settings**, **Remove** | **Start**, **Shut down**, **Force stop** | **Window**, **Full screen**), the VM list on the left with each VM's state, and the tabs **Details, Screen, Install, Host, GPU, Log** on the right. **Details** shows the VM in sections (General, System, Display, Storage, Audio, Network, USB, GPU passthrough) with a **Preview** of its screen. For a first install, use the order below.

### New VM

Click **New**. Like VirtualBox's wizard in expert mode, it has three folding sections:

1. **Virtual machine name and operating system:** the name, the VM's folder (by default `<Workspace>/<name>`; the Workspace is `~/macOS-VMs` unless you change it in **File › Preferences**) and the macOS version. Only Sonoma has been validated end to end.
2. **Specify virtual hardware:** Base Memory and Number of CPUs sliders, filled in with what is recommended for this host, and the display (in the app or a QEMU window).
3. **Specify virtual hard disk:** the disk size (thin qcow2, at least 40 GB) and whether to download the macOS recovery now.

**Finish** checks every section (it opens the one with a problem), makes the folder, extracts the boot files, creates the disk and, when asked, downloads and verifies the recovery. Progress shows in the Install tab.

The first time the app opens it adds a VM named "macOS Sonoma" with the defaults, so you can also go straight to Install. Only one VM runs at a time.

### Host

Make the checks green before starting: CPU virtualization, `/dev/kvm`, QEMU tools/version, CPU compatibility and `ignore_msrs`. Shut down running VirtualBox/VMware VMs. Click **Check again** after fixing a problem.

Click **Set up host** to authorize one operation that installs missing QEMU packages with `apt-get`, loads the appropriate KVM module if `/dev/kvm` is absent, creates/adds you to the `kvm` group, and enables/persists `ignore_msrs` in `/etc/modprobe.d/macos-kvm-gui.conf`. Log out/in if prompted so the new group applies, then recheck. The `libvirt` and `input` groups are not needed. Setup does not upgrade an already-present old QEMU or enable virtualization in BIOS.

**The password prompt.** Set up host runs through `pkexec`, so your desktop's polkit agent asks for **your user password** (the one you use with `sudo`). The app never sees or stores it. The prompt appears even if your `sudo` is configured without a password, because polkit does not read the sudoers file. The dialog shows the beginning of the command it will run (`/bin/sh -c case "$1" in …`); that is the app's fixed setup script, with your user name, the KVM module and the package list passed as separate, validated arguments. Cancelling the prompt changes nothing, and the app reports "Cancelled: nothing was changed." Showing a friendly text instead of the command would need a polkit policy installed as root, which the app does not do.

**Why not `libvirt` or `input`?** The app runs QEMU directly, so it never talks to libvirt. Keyboard and mouse reach the VM through the QEMU window, not through evdev. Membership in `input` also lets a user read every key typed on the machine, so do not add it unless another tool needs it.

If KVM loads from an initramfs, also rebuild that initramfs after the persistent setting; see [troubleshooting](troubleshooting.md#ignore_msrs).

### Settings

**Settings** (toolbar or **Machine › Settings…**) opens the selected VM's settings window, like VirtualBox's: categories on the left (**General, System, Display, Storage, Audio, Network, USB**) and the page on the right. Nothing is saved until **OK**. Defaults for a VM created on first run:

| Setting | Default |
| --- | --- |
| Folder | `<Workspace>/<name>`, for example `~/macOS-VMs/macos-sonoma` |
| macOS version | Sonoma |
| RAM | 8192 MiB |
| Cores / total vCPUs | 4 / 8 |
| SSH host port | 2222, bound to `127.0.0.1` |

**System** has the Base Memory and Processors sliders and **Threads per core**; with 2 threads, choose an even number of processors. The SSH port (in **Network**) must be 1024–65535. **Recommended for this host** fills in memory and processors but does not save them: click **OK**. Settings cannot be changed while the VM runs. **Storage**, **Audio** and **USB** show what the VM uses; the Install tab creates the disk and manages the install media.

**File › Preferences** holds the app-wide settings. The **Workspace** is the folder that holds one folder per VM; new and imported VMs go there. Changing it does not move existing VMs: **Machine › Move…** (or **Move…** in Settings › General) moves the selected VM's disk, install media, NVRAM, identity and boot files into a new folder (on the same disk) and leaves anything else in the old folder alone. **Theme** switches between Dark, Light and System right away.

An existing OSX-KVM folder can reuse `mac_hdd_ng.img` and `BaseSystem.img`. The app uses its own OpenCore and new SMBIOS identity, not the old boot files. Back up an existing VM first; iCloud/Messages may see a different Mac and require sign-in again.

### Install: steps 1–5

[![Installation journey](diagrams/install-flow.png)](diagrams/install-flow.svg)

1. **VM folder — Create VM folder.** Creates the folder and extracts the built-in boot files. An existing directory already counts as ready; missing boot files are prepared on Start. Status shows asset versions and their source.
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

### The VM screen

With the default **Display: In the app**, starting the VM switches to the **Screen** tab, which shows macOS inside the app (over VNC on a private socket, no TCP port).

- **Click the screen** to send the keyboard to macOS; it keeps the keyboard while it has focus. Keys travel as physical (US layout) keys, so keep macOS on the **ABC** or **U.S.** input source for an English keyboard.
- Only macOS's own pointer shows over the screen: the host pointer is hidden there.
- **Full screen** fills the monitor. **Ctrl+Alt+F** is the host key: it is never sent to macOS, it leaves full screen, and pressed again it closes the separate window and brings the screen back to the tab. **Window** opens the screen in its own resizable window.
- Choose **QEMU window** in Settings › Display to get QEMU's own window instead.

**Shut down** sends the ACPI power button, and the state stays Running until QEMU exits. Many macOS versions treat that button as Sleep, which does nothing in a VM, so shut macOS down from the **Apple menu › Shut Down** inside the VM; the app notices when it powers off. **Force stop** is like pulling the power cord and can lose data. The app blocks closing its window while the VM is running or paused.

### Optional tabs and next steps

- **GPU:** leave **None (emulated display)** selected for initial setup. [GPU passthrough](gpu-passthrough.md) is optional and **untested on real hardware**.
- **Log:** shows the last 1000 lines of app/QEMU output in this session. Use it with [troubleshooting](troubleshooting.md).
- Enable **Remote Login** inside macOS before connecting with `ssh -p 2222 user@127.0.0.1` (use your configured port/account).
- The emulated display has no Metal acceleration. Use a static wallpaper and Reduce motion/transparency; see [display limitations](troubleshooting.md#display-and-graphics).
- **File › Export VM…** writes the selected VM as a `.tar` with its settings, disk, NVRAM, identity and install media (stopped VMs only), or its settings alone as `.json`. **Import** adds a VM from either; a `.tar` is unpacked into a new folder inside the Workspace. A GPU address is never imported.
- **Remove** removes the VM from the list; deleting its files is a separate, confirmed choice and only touches the files the app created.
- The theme can also be forced with `MKG_THEME=dark|light|system`; see [settings and overrides](architecture.md#settings-and-overrides).

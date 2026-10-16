# Screenshots

Screens of macOS KVM with the default dark theme. They were captured on the reference host (Ubuntu 24.04, Ryzen 9 9900X, QEMU 8.2.2). Back to the [documentation](../README.md).

## Details

![Details tab](details.png)

The VirtualBox-style window: the toolbar (New, Edit, Delete, Export, Import | Start, Shut down, Force stop | Open in window, Full screen), the VM list on the left and the Details tab, with the VM's icon, a preview of its screen and its General, System, Display, Storage, Network and GPU settings.

## New VM wizard

![New VM wizard](new-vm.png)

Four steps: name and macOS version, hardware (recommended for this host), storage (the folder inside the Workspace, the disk size and whether to download the recovery now) and a summary. Create makes the folder and the disk and downloads the recovery.

## Install

![Install tab](install.png)

The guided install. **1 · VM folder** creates the folder and extracts the built-in OpenCore and OVMF. **2 · macOS recovery** downloads from Apple and verifies the signature. **3 · Disk** creates the thin qcow2. **4 · Install inside the VM** lists the steps to follow in macOS. **5 · Clean up** removes the install media. The status card shows the boot file versions and their source.

## Settings

![Settings tab](settings.png)

The selected VM's name, folder, macOS version, RAM, cores, vCPUs, SSH port and display, with **Move to Workspace**. Below are **Recommended for this host**, the Workspace (the folder that holds one folder per VM) and the theme.

## Host

![Host tab](host.png)

The host checks, each colored OK, Warning, Failure or Inconclusive. **Set up host** fixes the basics with one password prompt: missing QEMU packages, the KVM module, the kvm group and ignore_msrs. The home lab notice sits below.

## GPU

![GPU tab](gpu.png)

Every display adapter on the host, with its macOS compatibility, IOMMU group, driver and the reason it can or cannot be passed to the VM. On this host none is eligible: NVIDIA cards and the Ryzen iGPU have no macOS driver. See [GPU passthrough](../gpu-passthrough.md).

## Log

![Log tab](log.png)

The QEMU command line and output, together with the app's own messages (last 1000 lines).

## Updating these screenshots

Build with `make build`, run `./dist/macos-kvm-gui`, resize the window to 1280x820 and capture each tab. On X11, for example:

```sh
WID=$(xdotool search --name '^macOS KVM' | head -1)
xdotool windowsize --sync "$WID" 1280 820
import -window "$WID" docs/screenshots/<tab>.png
```

Keep personal paths and data out of the captures: run a separate instance with `XDG_CONFIG_HOME` pointing to an empty folder and a short Workspace such as `~/VMs` (`macos-kvm-gui workspace set ~/VMs`), so the captures show no personal VM.

# Screenshots

Screens of macOS KVM with the light theme. They were captured on the reference host (Ubuntu 24.04, Ryzen 9 9900X, QEMU 8.2.2). Back to the [documentation](../README.md).

## Details

![Details tab](details.png)

The manager, modelled on VirtualBox's: the File / Machine / Help menus, the toolbar (New, Import | Settings, Remove | Start, Shut down, Force stop | Window, Full screen), the VM list with each VM's state and the Details tab: General, System, Display, Storage, Audio, Network, USB and GPU passthrough sections, with the Preview of the screen beside General and System. The light theme uses VirtualBox's colors.

## New VM wizard

![New VM wizard](new-vm.png)

Like VirtualBox's wizard in expert mode: three folding sections for the name, folder and macOS version; the hardware (memory and CPU sliders, recommended for this host); and the hard disk (size and whether to download the recovery now). Finish makes the folder and the disk and downloads the recovery.

## Install

![Install tab](install.png)

The guided install. **1 · VM folder** creates the folder and extracts the built-in OpenCore and OVMF. **2 · macOS recovery** downloads from Apple and verifies the signature. **3 · Disk** creates the thin qcow2. **4 · Install inside the VM** lists the steps to follow in macOS. **5 · Clean up** removes the install media. The status card shows the boot file versions and their source.

## Settings

![Settings window](settings.png)

The VM's Settings window: General, System (Motherboard, Processor and Acceleration tabs), Display, Storage, Audio, Network and USB. Nothing is saved until OK. The Workspace and the theme are in File › Preferences.

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

Build with `make build`, run `./dist/macos-kvm-gui`, resize the window to 1280x820 and capture each tab (`MKG_THEME=light`). On X11, for example:

```sh
WID=$(xdotool search --name '^macOS KVM' | head -1)
xdotool windowsize --sync "$WID" 1280 820
import -window "$WID" docs/screenshots/<tab>.png
```

Keep personal paths and data out of the captures: run a separate instance with `XDG_CONFIG_HOME` pointing to an empty folder and a short Workspace such as `~/VMs` (`macos-kvm-gui workspace set ~/VMs`), so the captures show no personal VM.

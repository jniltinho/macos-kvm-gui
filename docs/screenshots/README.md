# Screenshots

Screens of macOS KVM with the light theme. They were captured on the reference host (Ubuntu 24.04, Ryzen 9 9900X, QEMU 8.2.2). Back to the [documentation](../README.md).

## Details

![Details tab](details.png)

The manager, modelled on VirtualBox's: the File / Machine / Help menus, the toolbar (New, Import | Settings, Remove | Start, Shut down, Force stop | Window, Full screen), the VM list with each VM's state and the Details tab: General, System, Display, Storage, Audio, Network, USB and GPU passthrough sections, with the Preview of the screen beside General and System. The light theme uses VirtualBox's colors.

## New VM wizard

![New VM wizard](new-vm.png)

Like VirtualBox's wizard in expert mode: four folding sections for the name, folder and macOS version; the hardware (memory and CPU sliders, recommended for this host); the network (adapter 1: NAT or bridged, adapter type, MAC address); and the hard disk (size and whether to download the recovery now). Finish makes the folder and the disk and downloads the recovery.

## Install

![Install tab](install.png)

The guided install as a stepper: the header shows how many of the five steps are done, and each step is a card with a badge (its number, a green check once done), its state (Done, Next, Waiting) and its action. The next step gets a blue stripe and the highlighted button; finished steps hide their controls. The download progress shows inside its step, and the steps to follow inside macOS fold away.

## Settings

![Settings window](settings.png)

The VM's Settings window: General, System (Motherboard, Processor and Acceleration tabs), Display, Storage, Audio, Network (Adapter 1–4 tabs: NAT or bridged, adapter type, MAC address) and USB. Nothing is saved until OK. The Workspace and the theme are in File › Preferences.

## Host

![Host tab](host.png)

The host checks in the same style: a summary and the **Check again** and **Set up host** actions at the top, then one card per check with a green, amber or red badge. **Set up host** fixes the basics with one password prompt: missing QEMU packages, the KVM module, the kvm group and ignore_msrs. The home lab notice sits below.

## GPU

![GPU tab](gpu.png)

The GPU for the VM and the VFIO reservation at the top, then one card per display adapter with its macOS compatibility, driver, IOMMU group and why it can or cannot be passed to the VM. On this host none is eligible: NVIDIA cards and the Ryzen iGPU have no macOS driver. See [GPU passthrough](../gpu-passthrough.md).

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

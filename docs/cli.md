# Command line

The same binary is a desktop app and a command line tool. Without a command it opens the GUI; with one it runs the same steps from a terminal, so installs can be scripted. Every command has `--help`.

```sh
macos-kvm-gui                 # opens the desktop app (same as `macos-kvm-gui gui`)
macos-kvm-gui --help          # all commands
macos-kvm-gui --vm "Tahoe lab" config show       # act on another VM than the selected one
macos-kvm-gui --config ./lab.json config show   # use a standalone settings file
```

VMs live in `~/.config/macos-kvm-gui/macos-kvm-gui.db` (SQLite). Commands act on the VM selected in the app (or with `vm select`), on `--vm <name or id>`, or on a standalone JSON file with `--config`. Each VM has its own folder, by default `<Workspace>/<name>`.

## Commands

| Command | What it does |
|---|---|
| `check` | Runs the Host tab's checks. Exits with an error when any check fails. |
| `setup` | Installs missing QEMU packages, loads the KVM module, adds you to the `kvm` group and enables `ignore_msrs`. It asks for your password once, through `pkexec`. |
| `config show` | Prints the settings as JSON. |
| `config set key=value...` | Changes settings. The keys are `workspace`, `version`, `ram`, `cores`, `vcpus`, `ssh-port`, `display` (`app` or `window`), `gpu`, and for network adapter N (1–4) `nicN.enabled` (`true`/`false`), `nicN.mode` (`nat` or `bridged`), `nicN.bridge`, `nicN.model` (`virtio-net-pci`, `vmxnet3`, `e1000-82545em`) and `nicN.mac` (an address, or `random`). Values go through the same validation as the app. |
| `config recommend` | Sets CPU and RAM to the values recommended for this host. |
| `workspace show` / `workspace set <dir>` | Prints or sets the Workspace, the folder that holds one folder per VM (default `~/macOS-VMs`). Existing VMs are not moved. |
| `workspace create` | Creates the VM's folder and extracts the built-in OpenCore and OVMF. |
| `download [--version sonoma]` | Downloads the macOS recovery from Apple, verifies its signature and converts it. |
| `disk create [--size 80]` | Creates the thin qcow2 disk, in GiB (minimum 40). It never overwrites an existing disk. |
| `media remove --yes` | Deletes `BaseSystem.img` once macOS boots from the disk. |
| `vm list` / `vm status [name]` | Lists the VMs (`*` marks the selected one) with their state, or shows one VM's state, PID, uptime and folder. VMs started outside the app, for example by OSX-KVM's script, show as running too. |
| `vm create <name> [--version v] [--folder dir]` | Adds a VM with CPU and RAM recommended for this host, selects it and creates its folder (default `<Workspace>/<name>`). |
| `vm select <name>` | Selects the VM the other commands and the app use. |
| `vm move <name> [folder]` | Moves the VM's disk, install media, NVRAM, identity and boot files to a new folder on the same disk (default `<Workspace>/<name>`). |
| `vm export <name> [file] [--full]` | Writes the settings as JSON, or with `--full` a `.tar` with the disk, NVRAM, identity and install media. |
| `vm import <file> [--folder dir]` | Adds a VM from a `.json` or a `.tar`; a `.tar` is unpacked into a new folder. |
| `vm delete <name> --yes [--purge]` | Removes the VM from the list; `--purge` also deletes the files the app created in its folder. |
| `vm start [--display app\|window] [-v]` | Starts the VM in the foreground and returns when QEMU exits. See below. |
| `gpu list` | Lists the display adapters, whether each can be passed to the VM, and why not. |
| `gpu reserve <pci-address>` / `gpu release` | Reserves an eligible GPU for `vfio-pci`, or releases it. Both need a host reboot. Experimental. |
| `version` | Shows the app version and the built-in boot file versions. |

### `vm start`

The command waits while the VM runs. The first **Ctrl+C** asks macOS to shut down (ACPI), and a second one forces the stop. With `display=app` there is no window: the command prints the private VNC socket, which any VNC viewer can open (for example `vncviewer /run/user/1000/macos-kvm-gui-*/vnc.sock`). With `--display window`, QEMU opens its own window. `-v` prints the QEMU command line and output.

## Automating an install

```sh
#!/usr/bin/env bash
set -euo pipefail

macos-kvm-gui setup || true            # may need a new login for the kvm group
macos-kvm-gui check                    # stops the script if the host is not ready

macos-kvm-gui vm create "Sonoma lab" --version sonoma   # recommended CPU/RAM, folder ~/macOS-VMs/sonoma-lab
macos-kvm-gui config set display=window
# optional: a second adapter on the host bridge br0 (see troubleshooting.md#bridged-networking)
macos-kvm-gui config set nic2.enabled=true nic2.mode=bridged nic2.bridge=br0 nic2.model=vmxnet3 nic2.mac=random
macos-kvm-gui download
macos-kvm-gui disk create --size 80

# Install macOS inside the QEMU window (Disk Utility → Erase as APFS → Reinstall macOS).
macos-kvm-gui vm start

# After macOS boots from the disk:
macos-kvm-gui media remove --yes
```

The steps inside the VM (erasing the disk and running the installer) stay manual.

## Exit status

Every command exits with `0` on success and `1` on an error, and prints the reason to stderr. `check` also exits with `1` when any check fails, which makes it a good first step in scripts.

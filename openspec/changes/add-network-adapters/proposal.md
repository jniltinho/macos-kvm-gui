# Proposal

## Why

Every VM gets one user-mode NAT adapter with a model picked by the macOS version and the same hard-coded MAC (`52:54:00:c9:18:27`, inherited from OSX-KVM). Labs need a VM reachable on the LAN (bridged), a second adapter, another adapter model when a macOS version lacks a driver, and a unique MAC so two VMs on one bridge do not collide.

## What Changes

- Up to four network adapters per VM, like VirtualBox's Adapter 1–4, each one:
  - enabled or not;
  - **attached to** `NAT` (QEMU user mode; the SSH port forward stays on the first NAT adapter, further NAT adapters get their own subnet) or `Bridged` to a host bridge through `qemu-bridge-helper` (the app and QEMU stay unprivileged; the helper must be setuid root, which Ubuntu does not do by default, and the bridge allowed in `/etc/qemu/bridge.conf`);
  - **adapter type** `virtio-net-pci` (Big Sur and later), `vmxnet3` (all versions) or `e1000-82545em` (through Monterey), defaulting to `vmxnet3` through Catalina and `virtio-net-pci` from Big Sur; this also fixes Mojave and Catalina, which got `virtio-net-pci` without a driver;
  - **MAC address**, a locally administered unicast address that can be edited or regenerated.
- The New VM wizard gets a "Specify network" section for adapter 1, and Settings › Network gets VirtualBox's Adapter 1–4 tabs; Details lists each enabled adapter; the CLI gets `config set nic<N>.<field>=value`.
- New VMs get a random MAC. Existing VMs with no adapter list keep today's single NAT adapter and its MAC, so macOS (iCloud, Messages) keeps seeing the same en0.
- Start refuses a bridged adapter whose bridge does not exist, whose helper is missing or that `bridge.conf` denies, and refuses an SSH port already in use; other helper failures come from QEMU's error with the fix. The app never edits `bridge.conf` or creates bridges. Adapters 2–4 sit on fixed PCI slots; with none enabled QEMU gets `-nic none`.
- Details also shows the disk's virtual size next to the space it uses.
- Out of scope: TAP devices that need root, host-only networks, NAT port forwards other than SSH, and USB network adapters.

## Capabilities

### New Capabilities
- `networking`: per-VM network adapters with their attachment, model and MAC.

## Impact

- `Config` gains `networks` (JSON); the SQLite `vms` table gains a `networks` column (`PRAGMA user_version = 2`). Bundles and settings exports carry it automatically.
- `BuildArgs` emits one `-netdev`/`-device` pair per enabled adapter.

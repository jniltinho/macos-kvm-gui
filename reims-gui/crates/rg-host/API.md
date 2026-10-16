# rg-host API

Port of the Go app's `internal/core/host.go`, `gpu.go` and `reimshost.go`. Synchronous, no
global state except the cached `apple-gfx-pci` probe.

## Host

`Host` is the system with every root and command swappable (tests use fake trees):

| Field | Real value |
| --- | --- |
| `proc_root`, `sys_root`, `etc_root`, `share_root` | `/proc`, `/sys`, `/etc`, `/usr/share` |
| `dev_kvm`, `dev_vfio`, `dev_udmabuf` | `/dev/kvm`, `/dev/vfio`, `/dev/udmabuf` |
| `look_path: LookPathFn` | `PATH` search |
| `output: OutputFn` | runs a program, combined output, `Err(CmdError)` on a non-zero exit |
| `run: RunFn` | runs a program, `CmdError { code, output }` (output on one line) |
| `reims_qemu: ReimsQemuFn` | **wire to the full lookup** (rg-reims/rg-core); the default only knows `MKG_REIMS_QEMU` and `~/.local/share/reims-ctl` |
| `extra_kexts: ExtraKextsFn` | **wire to settings.yaml's kexts** (count of entries, or the load error); the default returns 0 |
| `settings_file: String` | `~/.config/macos-kvm-gui/settings.yaml`, named in the patch-kext reason |

`Host::system()` builds the real one.

## Checks

- `Check { name, status, detail, hint }`, `Status::{Ok, Warn, Fail, Unknown}` (Display: OK,
  Warning, Failure, Inconclusive). Names, details and hints are the Go app's.
- `checks(version)`, `checks_for(&Config)`: CPU virtualization, /dev/kvm, qemu-system-x86_64,
  qemu-img, QEMU version (>= 8.2.2), CPU compatible with version, kvm ignore_msrs, Other
  hypervisors, then the Reims checks.
- `reims_checks_for(&Config)`: the Reims checks with Fail lowered to Warn unless the VM is on
  Reims vGPU (RAM 0 means the default 8192).
- `reims_checks(ram_mib)`: Reims vGPU QEMU (with `apple-gfx-pci`, probe cached by path+mtime),
  Vulkan driver, vm.max_map_count (`reims_map_count(ram) = ram*64 + 65536`, hint to Set up host
  and 2097152), /dev/udmabuf (AMD GPUs; warnings only).
- `reims_ready(&Config) -> Result<ReimsReady { qemu, warnings }, HostError>`: first Fail is the
  error; Warn/Unknown go to `warnings`.
- `udmabuf_hint()`, `udmabuf_exit(qemu_output) -> Option<String>`, `mem_total_mib()`.

## Setup (pkexec, constant scripts)

- `setup_host(user) -> Result<String, HostError>`: one prompt; installs missing QEMU packages,
  loads kvm_amd/kvm_intel, adds `user` to kvm, enables ignore_msrs, raises vm.max_map_count to
  2097152. Returns what the user still has to do.
- `fix_ignore_msrs() -> Result<(), HostError>`.
- `HostError::Cancelled` when the password prompt was dismissed (pkexec 126, or 127 with
  "Not authorized"); `HostError::Failed(msg)` otherwise.

## GPUs

- `gpus() -> Vec<Gpu>`: display adapters from sysfs with `compat: Compat`, `funcs: Vec<PciFunc>`
  (display function first), `eligible` and `reason` (same reasons as Go).
- `reserve_gpu(&Gpu)`, `release_gpu()`: vfio-pci reservation in /etc/modprobe.d and udev, via
  pkexec; `reserved_ids() -> String` ("" = none).
- `gpu_ready(&Config, extra_kexts, memlock) -> Result<Vec<String>, HostError>`: right before
  QEMU starts; `extra_kexts` is the start's settings.yaml snapshot (number of kexts listed),
  `memlock` the soft RLIMIT_MEMLOCK in bytes. Returns the slot's PCI addresses.
- `is_kepler("10de:128b")`: OCLP 2.5.1's Kepler IDs.

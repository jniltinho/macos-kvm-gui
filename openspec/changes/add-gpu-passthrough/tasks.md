# Tasks

## 1. Detection and eligibility

- [x] 1.1 Implement GPU listing from sysfs (class, IDs, group, driver, boot_vga, slot functions) with an injectable sysfs root; verify with a fake sysfs tree test
- [x] 1.2 Implement the compatibility table and the eligibility rules (IOMMU, boot display, connected monitor, last host GPU, non-GPU functions, every function's group, duplicate IDs); verify with table tests for each refusal reason
- [x] 1.3 Verify the listing on the reference host: three GPUs, all unsupported

## 2. Reservation and release

- [x] 2.1 Implement reserve and release through pkexec with a constant script, IDs passed as `$1`, Go-side ID validation and post-run verification; verify with the real shell rejecting hostile IDs and a simulated runner (success, cancel, invalid ID refused before elevation, no `blacklist` line)

## 3. Starting the VM with a GPU

- [x] 3.1 Add the optional `gpu` config field with validation; verify with config tests
- [x] 3.2 Add one `vfio-pci` device per slot function when a GPU is chosen, keeping `vmware-svga` as the input console; verify with argument tests
- [x] 3.3 Refuse to start when the GPU is no longer eligible, the macOS version is too old, a function is not bound to `vfio-pci`, a `/dev/vfio` node cannot be opened or the memlock limit is below the RAM; verify with tests for each message

## 4. Interface and documentation

- [x] 4.1 Add the GPU tab (listing, reasons, choose GPU, reserve, release, reboot notice); verify with a screenshot on the reference host
- [x] 4.2 Document GPU passthrough in the README (supported cards, BIOS primary display, reboot, reset bug, `agdpmod=pikera`, untested status); verify the README renders and its commands are correct

## 5. Integration

- [x] 5.1 Run `go vet ./...`, `go test -race ./...` and `openspec validate add-gpu-passthrough --strict` without errors

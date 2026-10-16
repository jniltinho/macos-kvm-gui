# Tasks

## 1. Setup

- [x] 1.1 Create `go.mod` (module `github.com/jniltinho/macos-kvm-gui`), add `fyne.io/fyne/v2` and verify that `go build ./...` passes
- [x] 1.2 Add `.gitignore` and `Makefile` (targets `build`, `test`, `run`) and verify that `make build` produces the binary

## 2. Configuration (vm-lifecycle)

- [x] 2.1 Implement `internal/core/config.go` with defaults, `~` expansion, validation (RAM, topology, port, path) and atomic JSON Load/Save; verify with round-trip, defaults and per-invalid-value tests
- [x] 2.2 Implement the per-version hardware profile (CPU, NIC, experimental); verify with a table test of the nine versions

## 3. Host checks (host-preflight)

- [x] 3.1 Implement the checks (CPU vmx/svm, avx2/sse4_1, `/dev/kvm`, tools, QEMU version, `ignore_msrs`, conflicting processes, inconclusive) with injectable `/proc` and `/sys` roots and executor; verify with tests over fake trees in `t.TempDir()`
- [x] 3.2 Implement the `ignore_msrs` fix via `pkexec` with a constant script, exit code translation and recheck; verify with a mock executor (success, 126, error, partial)

## 4. Media and disks (macos-install-media)

- [x] 4.1 Implement the per-workspace `flock` lock outside the workspace; verify with a test of two concurrent acquisitions and that an empty workspace stays empty
- [x] 4.2 Implement workspace classification and the shallow clone; verify the classification (empty, ready, incomplete) with a test
- [x] 4.3 Implement chunklist verification (header, RSA signature, hashes, trailing bytes); verify with the real Sonoma chunklist and a synthetic tampered/truncated DMG
- [x] 4.4 Implement the staged download, progress parsing with `\r` and the stages; verify the parsing with a fragmented stream in a test
- [x] 4.5 Implement atomic conversion, version recording, staging cleanup and the 4 GiB free space check; verify with a test that a failed conversion preserves the previous media and that an old marker does not survive publishing
- [x] 4.6 Implement disk creation (minimum 40 GiB, no overwrite, space warning) and media removal with bytes freed; verify with real `qemu-img` in `t.TempDir()`

## 5. VM lifecycle (vm-lifecycle)

- [x] 5.1 Implement QEMU argument building (profile, `,,`, hostfwd on 127.0.0.1, smp, with and without media, own NVRAM); verify with table tests
- [x] 5.2 Implement the QMP client (greeting, capabilities, STOP/RESUME/BLOCK_IO_ERROR events by action, powerdown, deadlines, disconnect); verify with a fake QMP server in a test
- [x] 5.3 Implement start, one-time VARS copy, Pdeathsig with the creating thread pinned (LockOSThread), log capture, kill and translation of the "KVM busy" error; verify with translation and no-VARS-overwrite tests, and with a real start on the reference host

## 6. Desktop interface

- [x] 6.4 Implement the recommended resources calculation (host threads and MemTotal) and the "Recommended for this host" button in Settings; verify with a table test (reference host and small host)
- [x] 6.5 Show video performance guidance in the VM tab; verify with a screenshot

- [ ] 6.1 Implement the Fyne window with Host, Install, VM and Log tabs, operations in goroutines and updates via `fyne.Do`; verify by opening the app and running each action on the reference host
- [ ] 6.2 Show the manual installation steps, confirm force stop and media removal, and intercept closing while the VM is running; verify manually
- [x] 6.3 Write the README (build/runtime dependencies, build, usage, limitations such as initramfs and experimental versions); verify that the documented commands work

## 7. Integration

- [x] 7.1 Run `go vet ./...`, `go test -race ./...` and `openspec validate add-desktop-gui --strict` with no errors

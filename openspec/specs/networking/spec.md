# networking Specification

## Purpose
Lets each VM have up to four network adapters, NAT or bridged, with a chosen model and its own MAC address.

## Requirements

### Requirement: Adapters
The system SHALL support up to four adapters per VM, each enabled or disabled, attached to NAT or to a host bridge, with an adapter type of `virtio-net-pci`, `vmxnet3` or `e1000-82545em` and a MAC address, and SHALL start QEMU with one network device per enabled adapter.

#### Scenario: Second adapter on a bridge
- **WHEN** adapter 1 is NAT and adapter 2 is bridged to `br0` with type `vmxnet3`
- **THEN** QEMU gets a user-mode netdev for adapter 1 and a bridge netdev on `br0` with a `vmxnet3` device for adapter 2

### Requirement: Stable device placement
The system SHALL keep adapter 1 in its place among the other devices, SHALL put adapters 2–4 on fixed PCI slots so adding one does not move other devices, and SHALL start QEMU with `-nic none` when no adapter is enabled.

#### Scenario: No adapter
- **WHEN** every adapter is disabled
- **THEN** QEMU gets `-nic none` and no default network device

### Requirement: Adapter type defaults
The system SHALL default new adapters to `vmxnet3` through Catalina and `virtio-net-pci` from Big Sur, and SHALL warn in the settings about a type the VM's macOS version has no built-in driver for.

#### Scenario: Intel adapter on Sonoma
- **WHEN** the user picks Intel PRO/1000 MT Desktop for a Sonoma VM
- **THEN** the settings warn that macOS Sonoma has no driver for it, and still allow saving

### Requirement: Existing VMs keep their network
The system SHALL treat a VM without an adapter list as one NAT adapter with the macOS version's default type and the MAC address used so far, and the database migration SHALL leave such VMs without a list.

#### Scenario: Upgrade
- **WHEN** a Sonoma VM created before this change starts
- **THEN** its QEMU network arguments are the same as before (Mojave and Catalina VMs move from `virtio-net-pci`, which they had no driver for, to `vmxnet3`)

### Requirement: MAC addresses
The system SHALL accept only unicast, locally administered MAC addresses written as six hexadecimal pairs, SHALL refuse two adapters of one VM with the same MAC, and SHALL give each new VM and each newly enabled adapter a random MAC generated once and stored. Imports SHALL keep the MAC addresses they carry.

#### Scenario: Multicast address
- **WHEN** the user enters `01:00:5e:00:00:01`
- **THEN** the settings are not saved and the error names the adapter and the MAC

### Requirement: Bridged adapters without root
The system SHALL attach bridged adapters through QEMU's `qemu-bridge-helper`, keeping the app and QEMU unprivileged, and SHALL refuse to start when the bridge does not exist, the helper is missing or, when the file is readable, `/etc/qemu/bridge.conf` denies the bridge under the helper's rules (`allow`, `deny` and `include`, a matching `deny` winning). Other helper failures SHALL be reported from QEMU's error with the fix. The system SHALL NOT change persistent host network configuration.

#### Scenario: Bridge not allowed
- **WHEN** adapter 2 is bridged to `br0` and `/etc/qemu/bridge.conf` contains `deny br0`
- **THEN** Start fails with a message that tells the user to allow `br0` in `/etc/qemu/bridge.conf`

#### Scenario: Helper without privileges
- **WHEN** `qemu-bridge-helper` is not setuid root
- **THEN** Start fails with a message that explains the helper needs setuid root or `CAP_NET_ADMIN`

### Requirement: SSH forward
The system SHALL forward the SSH port from `127.0.0.1` to guest port 22 on the first enabled NAT adapter only, SHALL give further NAT adapters their own subnet, SHALL refuse to start when the port is already in use, and SHALL show that no forward exists when no adapter uses NAT.

#### Scenario: Bridged only
- **WHEN** the only enabled adapter is bridged
- **THEN** QEMU gets no `hostfwd` and Details says to use the VM's address on its network, if reachable

### Requirement: A free SSH port for new VMs
When a VM is created or imported (New VM wizard, `vm create`, the MCP server's new `vm_create`, `vm import`, the GUI's Import), in either app, without an explicit port, it SHALL get the lowest port from 2222 up that no other VM in the VM list uses and that is free on 127.0.0.1. The wizard SHALL pre-fill that port and let the user change it. If no such port exists, creation SHALL fail with an error and leave nothing behind.

#### Scenario: Second VM
- **WHEN** a VM already uses port 2222 and the user creates another VM without choosing a port
- **THEN** the new VM gets 2223 (or the next port that is free on 127.0.0.1 and unused by any VM)

#### Scenario: Import of a VM with a taken port
- **WHEN** a VM exported with port 2222 is imported while another VM uses 2222
- **THEN** the imported VM gets the next free port, and the app says which port it got and why

### Requirement: Requested port already taken
A port requested explicitly (`vm create --ssh-port`, the wizard's field, an imported VM's stored port) that no other VM uses SHALL be kept, even when another process holds it on 127.0.0.1. If it belongs to another VM, the new VM SHALL get the next free port instead, and the system SHALL report it: a log line in the GUI, a line in the CLI output, the port in the MCP result.

#### Scenario: Explicit unused port
- **WHEN** `vm create --ssh-port 2300` runs and no VM uses 2300
- **THEN** the VM gets 2300

#### Scenario: Out of range
- **WHEN** `vm create --ssh-port 80` runs
- **THEN** creation fails with "use a port between 1024 and 65535" and no VM is added

#### Scenario: Explicit port of another VM
- **WHEN** `vm create --ssh-port 2222` runs while VM "Nilton" uses 2222
- **THEN** the VM is created with the next free port and the command prints that 2222 belongs to "Nilton"

### Requirement: Free port button
Settings › Network SHALL offer, next to the SSH port field in both apps, a "Use a free port" button that fills in the lowest port from 2222 up that no other VM uses and that is free on 127.0.0.1.

#### Scenario: Pick a free port
- **WHEN** the user clicks "Use a free port" for "sonoma-test" while "Nilton" uses 2222
- **THEN** the field shows 2223 (or the next free port), and the shared-port warning goes away

### Requirement: Shared port warnings
Editing a VM's SSH port to one that another VM uses (Settings › Network, `config set ssh-port=`) SHALL warn and name that VM, and SHALL still save. While two VMs share a port, Details SHALL show the warning for each of them. Existing VMs SHALL NOT be renumbered automatically.

#### Scenario: Settings
- **WHEN** the user sets VM "sonoma-test" to port 2222 in Settings › Network while "Nilton" uses 2222
- **THEN** Settings shows "Also used by Nilton: they cannot run at the same time", OK saves, and Details for both VMs shows the warning

#### Scenario: Existing duplicates stay
- **WHEN** the app opens a VM list where two VMs already share 2222
- **THEN** both keep 2222, Details warns, and starting the second while the first runs fails with the existing "SSH port … is in use" message

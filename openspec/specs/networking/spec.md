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

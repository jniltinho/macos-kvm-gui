# Tasks

## 1. Core
- [x] 1.1 `NIC` type (enabled, mode, bridge, model, MAC) and `Config.Networks`, validation (MAC format/unicast/unique, model list, bridge name)
- [x] 1.2 Effective adapters: an empty list means today's single NAT adapter with the profile's model and the legacy MAC
- [x] 1.3 QEMU arguments: user or bridge netdev per adapter; SSH forward on the first NAT adapter
- [x] 1.4 Bridge checks before start: bridge exists, `qemu-bridge-helper` present, allowed in `/etc/qemu/bridge.conf`
- [x] 1.5 Database column and migration (user_version 2); random MAC for new VMs

## 2. GUI and CLI
- [x] 2.1 Settings › Network: Adapter 1–4 tabs (enable, attached to, bridge, adapter type, MAC with regenerate)
- [x] 2.2 Details › Network lists the enabled adapters
- [x] 2.3 `config set nic<N>.enabled|mode|bridge|model|mac`

## 3. Tests and docs
- [x] 3.1 Tests for arguments, validation, legacy defaults and migration
- [x] 3.2 Docs (getting started, CLI, troubleshooting for bridges)

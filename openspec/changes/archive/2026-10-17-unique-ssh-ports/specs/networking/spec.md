# Spec Delta

## ADDED Requirements

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

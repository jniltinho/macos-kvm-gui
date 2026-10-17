# mcp-server Specification

## Purpose
Lets AI agents control the app's VMs over the Model Context Protocol: see the screen, type and click, start and stop VMs and install macOS from the offline installer.

## Requirements

### Requirement: Local MCP server
The system SHALL provide `macos-kvm-gui mcp`, an MCP server over standard input and output only that writes nothing but MCP messages to standard output, and SHALL NOT open any network listener.

#### Scenario: Agent registration
- **WHEN** an agent starts `macos-kvm-gui mcp` and lists its tools
- **THEN** it gets the VM, screen, input, installer and host tools with their input schemas

### Requirement: Run ownership and discovery
The system SHALL let one process own a running VM and one other process attach to it through the run's VNC socket and a second QMP socket, SHALL find a run only as the QEMU process that has the VM's disk open, using its sockets only inside this user's private run directories, and SHALL NOT stop a VM when an attached process or the MCP server exits.

#### Scenario: VM started in the app
- **WHEN** a VM runs in the app with Display: In the app and an agent calls `screen_capture` for it
- **THEN** the agent gets the current guest screen as a PNG image and the app keeps showing it

#### Scenario: VM started by the agent
- **WHEN** an agent starts a VM with `vm_start` and then the agent exits
- **THEN** the VM keeps running and the app can show it

#### Scenario: Socket outside a private directory
- **WHEN** a QEMU process opens the VM's disk but its sockets are in a directory another user owns or that is not 0700
- **THEN** no tool connects to them

### Requirement: Screen and input
The system SHALL return the guest screen as PNG image content in guest coordinates (with the scale when reduced) only after a complete frame, SHALL send key chords and pointer events serialized per run with pauses between events, and SHALL refuse text the run's keyboard layout table does not cover before sending any key.

#### Scenario: Menu bar from the keyboard
- **WHEN** an agent sends `ctrl+f2`, `right` four times, `down`, `t` and `return` while the recovery shows its window
- **THEN** the recovery opens Utilities › Terminal

#### Scenario: Untypeable text
- **WHEN** the run's keyboard layout is Dvorak and an agent calls `type_text` with `sh install`
- **THEN** the tool fails with a message and no key is sent

#### Scenario: VM in its own window
- **WHEN** a VM runs with Display: In a window and an agent calls `screen_capture`
- **THEN** the tool fails saying screen and input need Display: In the app

### Requirement: Consent flags
The system SHALL require `confirm: true` for a forced stop and for deleting an installer, and both `accept_license: true` and `erase_disk: true` for an unattended offline install, SHALL check them and every prerequisite before any side effect, and SHALL mark these tools as destructive.

#### Scenario: Unattended without license consent
- **WHEN** an agent calls `install_offline` with `unattended: true` and `erase_disk: true` but no `accept_license`
- **THEN** nothing is typed and the error says that the unattended install accepts Apple's macOS license for the user

### Requirement: Download progress
The system SHALL send progress notifications for `installer_download` only when the request carries a progress token, with one value that never decreases across stages, and SHALL keep the partial download when the request is cancelled.

#### Scenario: Cancel during the check
- **WHEN** an agent cancels `installer_download` while the package is being checked
- **THEN** the downloaded package is kept and the next download resumes from it

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

### Requirement: Read VM settings
The MCP server of both apps SHALL offer `vm_config_get`, which returns a VM's settings: name, version, RAM, cores, vCPUs, SSH port, graphics, resolution, display, GPU, root-patch and network adapters.

#### Scenario: Read the port
- **WHEN** an agent calls `vm_config_get` with `vm: "Nilton"`
- **THEN** the result includes `ssh_port: 2222` and the VM's other settings

### Requirement: Change VM settings
The MCP server of both apps SHALL offer `vm_config_set`, taking the CLI's `config set` keys `ssh-port`, `ram`, `cores`, `vcpus`, `graphics`, `resolution`, `display`, `gpu` and `version`, with the CLI's values and validation, and returning the new settings with any warnings. `root-patch` and `workspace` SHALL be refused (GUI and CLI only: they need confirmations and downloads). It SHALL refuse while the VM runs, and SHALL refuse what the CLI refuses, with the same message.

#### Scenario: Change the SSH port
- **WHEN** an agent calls `vm_config_set` with `vm: "sonoma-test"` and `ssh-port: 2223` while the VM is stopped
- **THEN** the VM's port is 2223 in both apps, and the result carries a warning if another VM also uses 2223

#### Scenario: Root-patch refused
- **WHEN** an agent calls `vm_config_set` with `root-patch: false`
- **THEN** nothing changes and the error says to use the app or `config set`

#### Scenario: Running VM
- **WHEN** an agent calls `vm_config_set` on a running VM
- **THEN** nothing changes and the error says to shut the VM down first

### Requirement: Create a VM
The MCP server of both apps SHALL offer `vm_create`, which does what `vm create` does: a VM with the host's recommended resources, a name, fresh network MACs, an optional explicit SSH port, its folder and disk prepared and the VM selected. It SHALL return the VM with the SSH port it got, and the VM SHALL start with `vm_start` without further steps.

#### Scenario: Create with 2222 taken
- **WHEN** an agent calls `vm_create` with `name: "lab"` and `version: "sonoma"` while another VM uses 2222
- **THEN** the VM is created with the next free SSH port, and the result names that port

### Requirement: Run a command in the guest over SSH
The MCP server of both apps SHALL offer `vm_ssh`, which runs one command in a running VM's macOS over its SSH forward (127.0.0.1 and the VM's SSH port) and returns standard output, standard error and the exit code, with a timeout. It SHALL authenticate with the user's SSH keys or agent, or with a `password` given in the call, which it SHALL NOT log, store or pass on a command line. It SHALL require `confirm: true`, be marked destructive, and refuse a VM that is not running or has no SSH forward.

#### Scenario: Run a command
- **WHEN** an agent calls `vm_ssh` with `vm: "Nilton"`, `user: "nilton"`, `command: "sw_vers -productVersion"` and `confirm: true` while the VM runs
- **THEN** the result has `stdout: "15.8.1"` and `exit_code: 0`

#### Scenario: Without consent
- **WHEN** an agent calls `vm_ssh` without `confirm: true`
- **THEN** nothing connects and the error says the tool runs commands in the guest

#### Scenario: VM stopped
- **WHEN** an agent calls `vm_ssh` for a stopped VM
- **THEN** the error says the VM is not running

# Spec Delta

## ADDED Requirements

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

# Spec Delta

## MODIFIED Requirements

### Requirement: Display
A `reims-vgpu` VM's screen SHALL be Reims vGPU's own window (QEMU with `-display none`, `REIMS_VGPU_WINDOW` not set to 0), whatever the VM's Display setting. When the Go app starts the VM and can embed it (see the `screen-embedding` capability), that window SHALL be shown inside the Screen tab. Otherwise it SHALL stay on the desktop, and the Screen tab SHALL NOT attach as the interactive screen and SHALL say that the screen is in Reims vGPU's window. In both cases the app's VNC socket SHALL still be served: the Preview and the MCP server's screen capture and input SHALL keep working. This SHALL hold for VMs started by the app and by another process (CLI, MCP); only the app's own starts are embedded.

#### Scenario: Start from the app
- **WHEN** a `reims-vgpu` VM starts from the app on X11 with embedding available
- **THEN** the guest shows inside the Screen tab, and the Preview updates

#### Scenario: Start from the app without embedding
- **WHEN** a `reims-vgpu` VM starts from the app with no X11 or without the embedding binding
- **THEN** a Reims vGPU window shows the guest, the Preview updates, and the Screen tab says the screen is in that window

#### Scenario: Started by the MCP server
- **WHEN** an MCP client starts a `reims-vgpu` VM and calls `screen_capture`
- **THEN** the capture shows the guest, and the app shows the VM running with its Preview, without embedding or attaching the Screen tab

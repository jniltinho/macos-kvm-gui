# embedded-display Specification

## Purpose
Shows the macOS VM's screen inside the app, with keyboard and mouse, so the VM can be used without a separate QEMU window.

## Requirements

### Requirement: Private display export
The system SHALL, when the Display setting is "In the app", start QEMU without its own window and with VNC on a Unix socket inside the private, user-only runtime directory of that VM run, and SHALL never open a VNC TCP port.

#### Scenario: Default start
- **WHEN** the VM starts with the default settings
- **THEN** QEMU has `-display none` and `-vnc unix:<private dir>/vnc.sock`, and no QEMU window opens

#### Scenario: QEMU window fallback
- **WHEN** the Display setting is "QEMU window"
- **THEN** QEMU starts with its own window and without VNC, as before

### Requirement: In-app screen
The system SHALL show the VM's screen in the VM tab while the VM runs, updated as the guest redraws, scaled to fit the available area without distortion, and SHALL show a placeholder when the VM is stopped.

#### Scenario: VM running
- **WHEN** the VM is running and the guest draws its screen
- **THEN** the VM tab shows the same image the guest shows, and it follows changes of the guest's resolution

#### Scenario: VM stops
- **WHEN** QEMU exits
- **THEN** the screen is replaced by a placeholder and the viewer releases its connection

### Requirement: Keyboard and mouse input
The system SHALL send keyboard and mouse input to the VM while the screen has focus. Clicking the screen gives it focus. Pointer positions SHALL map to guest coordinates through the current scaling, and keys still held when focus is lost SHALL be released in the guest.

#### Scenario: Typing
- **WHEN** the screen has focus and the user types letters, digits, Shift, Ctrl, Alt (Option), Super (Command), arrows, Return, Escape or Backspace
- **THEN** the guest receives the matching key presses and releases

#### Scenario: Clicking a scaled screen
- **WHEN** the screen is drawn at half size and the user clicks its center
- **THEN** the guest receives the click at the center of its framebuffer

#### Scenario: Focus lost with a key down
- **WHEN** the user switches away while holding Shift on the screen
- **THEN** the guest receives the Shift release

### Requirement: Separate window
The system SHALL let the user open the screen in a separate, resizable window, and SHALL return it to the VM tab when that window closes.

#### Scenario: Detach and close
- **WHEN** the user opens the separate window and then closes it
- **THEN** the screen keeps working in the window while it is open, and shows again in the VM tab after it closes

### Requirement: Full screen and host key
The system SHALL offer a full screen view of the VM's screen. Ctrl+Alt+F SHALL be the host key: it is never sent to the guest, the Ctrl and Alt presses already sent are released, it leaves full screen, and in the separate window it closes the window and returns the screen to the VM tab.

#### Scenario: Enter and leave full screen
- **WHEN** the user clicks Full screen and then presses Ctrl+Alt+F
- **THEN** the screen fills the monitor, then returns to a normal window, and the guest never receives the F key

#### Scenario: Back to the tab
- **WHEN** the separate window is open and not in full screen, and the user presses Ctrl+Alt+F
- **THEN** the window closes and the screen shows again in the VM tab

### Requirement: Single pointer
The system SHALL hide the host pointer over the screen while a VM is shown, because macOS draws its own pointer into the framebuffer, and SHALL not resend unchanged pointer states.

#### Scenario: Pointer over the screen
- **WHEN** the user moves the mouse over the running VM's screen
- **THEN** only the guest's pointer is visible

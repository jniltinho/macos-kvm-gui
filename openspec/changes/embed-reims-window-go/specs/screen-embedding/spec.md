# Spec Delta

## Purpose

How the Go app hosts a QEMU display window, Reims vGPU's own window, inside its Screen tab on X11, and how it falls back to that window on the desktop when it cannot.

## ADDED Requirements

### Requirement: When the window is embedded
The Go app SHALL embed a Reims vGPU VM's window in the Screen tab only when all of these hold: the app itself started the VM, the app's window is on X11 (directly or through XWayland), and the app was built with embedding. Otherwise the VM SHALL keep Reims vGPU's own window on the desktop and the Screen tab SHALL say where the screen is. Embedding SHALL never take a window that belongs to another process's VM or to another reims-gui or macos-kvm-gui instance.

#### Scenario: Started from the app on X11
- **WHEN** a Reims vGPU VM starts from the Go app whose window is on X11
- **THEN** within 30 seconds macOS shows inside the Screen tab and no separate Reims vGPU window remains on the desktop

#### Scenario: Started by the CLI
- **WHEN** `macos-kvm-gui vm start` starts the VM while the app is open
- **THEN** the guest stays in Reims vGPU's own window, the app shows it as running outside the app, and the Preview updates

#### Scenario: Built without embedding
- **WHEN** the app was built without the embedding binding and starts a Reims vGPU VM
- **THEN** the VM behaves exactly as before this change: its own window, the Preview, and the Screen tab's notice

### Requirement: Geometry follows the Screen tab
The embedded window SHALL cover exactly the Screen tab's area, in physical pixels at the window's scale, and SHALL follow it when the app window moves, resizes, changes monitor or scale, and when the guest changes its resolution. The guest image SHALL fill the area as Reims vGPU draws it, with no offset.

#### Scenario: Resize and HiDPI
- **WHEN** the app window is resized, or moved to a monitor with a different scale
- **THEN** the embedded window's position and size match the Screen tab's rectangle within one frame of the change, with no gap or overlap

### Requirement: Hidden under overlays and other tabs
While a dialog, a menu of the app's menu bar, or any other popup covers the Screen tab, or another tab is selected, or the app window is minimized, the embedded window SHALL be hidden, so that the app's UI is never drawn under the guest and clicks never reach macOS. It SHALL reappear when the tab is visible and uncovered again.

#### Scenario: Dialog over the screen
- **WHEN** Settings opens while the guest is on screen
- **THEN** the dialog is fully visible and usable, and the guest shows again when the dialog closes

#### Scenario: Menu over the screen
- **WHEN** the File menu opens while the guest is on screen
- **THEN** the whole menu is visible and its items can be chosen, and the guest shows again when the menu closes

### Requirement: Keyboard to the guest
A click on the embedded guest, or the pointer entering it while the app is the active application, SHALL give the guest the keyboard. Ctrl+Alt+Super SHALL release the guest's capture, as in Reims vGPU's own window.

#### Scenario: Typing in the guest
- **WHEN** the user clicks the guest and types
- **THEN** the keys reach macOS

### Requirement: Keyboard back to the app
A click on the app's own UI SHALL give the keyboard back to the app, which SHALL keep it until the pointer enters the guest again or the guest is clicked. The guest SHALL never take the keyboard while another application is in front, including from focus events queued before the switch.

#### Scenario: Back to the app
- **WHEN** the guest has the keyboard and the user clicks the VM list
- **THEN** the arrow keys move in the VM list, not in macOS

#### Scenario: Another application in front
- **WHEN** the user switches to another application right after clicking the guest
- **THEN** that application keeps the keyboard

### Requirement: Release on signals
When the app gets SIGTERM, SIGINT or SIGHUP with a guest embedded, it SHALL put the window back on the desktop before it exits; QEMU then stops with the app, as it does today. The app SHALL still refuse to close normally while a VM runs.

#### Scenario: Signal with a VM on screen
- **WHEN** the app gets SIGTERM while a Reims vGPU guest is embedded
- **THEN** the window is released before the app exits, and the X server and the desktop stay up

### Requirement: Fallback when embedding fails
If the window cannot be found or embedded within 30 seconds, the VM SHALL keep running in its own window and the app SHALL log why.

#### Scenario: Embedding fails
- **WHEN** the Reims vGPU window does not appear within 30 seconds
- **THEN** the VM keeps running in its own window and the Log tab says embedding was skipped and why

### Requirement: Stop and restart
When QEMU exits, the embedding SHALL be released and the Screen tab SHALL show its stopped placeholder, and a later start SHALL embed again. A window lookup still running for an earlier run SHALL never embed a window into a later one.

#### Scenario: Restart
- **WHEN** the embedded VM shuts down and is started again from the app
- **THEN** the new window is embedded again

#### Scenario: Stop during the lookup
- **WHEN** the VM is stopped and started again before the first run's window lookup finishes
- **THEN** only the second run's window is embedded, and nothing is left over from the first

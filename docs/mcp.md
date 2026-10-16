# MCP server for AI agents

`macos-kvm-gui mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server. With it, an AI agent such as Claude Code or Codex can see a VM's screen, type and click in it, start and stop it, and install macOS from the offline installer, so you can ask the agent to install or check a VM and watch what it does in the app.

It talks over standard input and output only and opens no network port. It drives VMs through the same private sockets the app uses, in your user's 0700 runtime directory.

## Setup

Register it once in your agent:

```sh
claude mcp add macos-kvm -- macos-kvm-gui mcp     # Claude Code
codex mcp add macos-kvm -- macos-kvm-gui mcp      # Codex
```

Then ask, for example: "Install Sequoia on my VM offline and tell me when it reaches Setup Assistant."

## Tools

| Tool | What it does |
| --- | --- |
| `vm_list`, `vm_status` | The VMs; for one, QEMU's state, whether the offline disc is attached, the keyboard layout the recovery uses and whether the screen is available. `guest_phase` is always `unknown`: the server does not read the screen, the agent does. |
| `vm_start` | Starts a VM (optionally with `verbose_boot`) with its screen available. It first downloads the [extra kexts](architecture.md#extra-kexts) in `settings.yaml` that are not cached yet, so the start timeout covers only QEMU. The VM runs in its own process, so it keeps running when the agent exits, and the app shows it within a few seconds. |
| `vm_shutdown` | Presses the VM's power button: macOS shuts down by itself, or asks. Setup Assistant and the recovery may ignore it. |
| `vm_force_stop` | Stops the VM at once, losing unsaved data. Needs `confirm: true`. |
| `screen_capture` | The guest screen as a PNG image (scaled to `max_width`, default 1280), with its size and scale. |
| `send_keys` | Key chords in order, such as `ctrl+f2` (focus the menu bar), `down`, `return`, `cmd+q`. Names are U.S. key positions; `kp_divide` and the other `kp_` keys are the keypad's. |
| `type_text` | Text as physical keys in the VM's keyboard layout (U.S. or Brazilian ABNT2). It never presses Return. Characters the layout cannot type (ABNT2's dead keys and its `?`) are refused before any key is sent. |
| `pointer` | `move`, `click`, `double_click`, `right_click`, `scroll_up`, `scroll_down` at guest coordinates. |
| `installer_list`, `installer_download`, `installer_remove` | The offline installers. A download reports progress, resumes, and must finish before the VM starts so the VM gets the disc. Removing needs `confirm: true`. |
| `install_offline` | What **Type it in the VM** does: opens the recovery's Terminal and types the install line. `unattended: true` erases the VM's empty disk and accepts Apple's macOS license for you, so it also needs `erase_disk: true` and `accept_license: true`. |
| `host_check` | The Host tab's checks. |

Screen and input need **Display: In the app** (the default); a VM in QEMU's own window has no screen socket.

## Safety

- The `confirm`, `erase_disk` and `accept_license` flags and the tools' "destructive" marks are guardrails for agents that follow them. They are set by the agent, so they do not prove that you agreed: an agent with `send_keys` and `pointer` can click anything you could. Run the server only for agents you want controlling your VMs, and answer their questions about erasing disks and Apple's license yourself.
- Keys pressed while macOS boots change how it boots. Agents should wait until `screen_capture` shows the recovery's window before `install_offline`.
- Do not type in the VM while an agent does: the keys would mix.
- A VM started by the app can be driven by the server, and one started by the server is shown by the app. Each run has one extra control connection, so one agent at a time.

# Parity with the Go app

The checklist of every feature of the Go app (`cmd/`, `internal/mcp`, `internal/gui`,
`internal/core`) and its counterpart in `reims-gui` (design D1, spec "Feature parity"). The Go
app stays until every line is checked (design, Migration Plan).

Each line: **Go feature** — Rust counterpart — how it is verified (a test name, run with
`scripts/cargo-docker.sh cargo test --workspace`; a golden file; or `manual: <check>`).

- `[x]`: the Rust code exists (read in the crates) and the named test or manual check covers it.
- `[ ]`: missing or pending; the line says what is left.

Status as of branch `feat/reims-gui-rust` after commit `8b9a6b9` (Screen embedding, VNC screen,
file dialogs) and the Go-parity follow-up (close guard, disk size, offline install box, root-patch
confirmations). What remains is the rest of the live checks (task 5.2) and the manual GNOME session checks (task 5.2b; the automated part passes on both releases).

## 1. Command line (`cmd/` → `rg_agent::cli`)

Same names, flags, output and exit status; tables like Go's tabwriter
(`table_pads_every_cell_but_the_last`). Parse errors: `unknown_commands_and_flags_are_errors`.

### Global

- [x] `--config <file.json>` (one VM file instead of the list) — `rg_agent::cli::Cli::config` — `real::tests::a_config_file_is_the_only_vm` (GUI), manual: `reims-gui --config vm.json config show`
- [x] `--vm <name|id>` — `rg_agent::cli::Cli::vm`, `rg_core::registry::find_vm` — `vm_commands`, `find_vm_prefers_the_id`
- [x] No command or `gui` opens the GUI — `rg_agent::cli::main` (`Cmd::Gui`), `apps/reims-gui/src/main.rs` — `self_test_draws_frames_under_xvfb`
- [x] `--version`, `version` (app version and boot files) — `Cmd::Version`, `rg_assets::asset_versions` — `config_commands` (runs `version`)
- [x] Hidden `--self-test` (MCP initialize + tools/list, then GUI frames) — `rg_agent::self_test`, `rg_ui::RunOptions::self_test` — `self_test_lists_the_tools`, `self_test_draws_frames_under_xvfb`, `scripts/check-package.sh`

### Commands

- [x] `check` — `Cmd::Check` on `rg_host::Host::checks_for` — `checks_ready_host_are_all_ok`, `checks_report_problems`; manual: `reims-gui check` on this host prints the same rows as `macos-kvm-gui check`
- [x] `setup` (pkexec, one prompt) — `Cmd::Setup` on `rg_host::Host::setup_host` — `setup_host_runs_the_script_with_checked_arguments`, `setup_script_rejects_injection`, `pkexec_cancel_is_told_apart`
- [x] `mcp` — `rg_agent::serve_stdio` — `tools`, `self_test_lists_the_tools`
- [x] `config show` — `ConfigCmd::Show` — `config_commands`, `json_matches_go`
- [x] `config set key=value...` (workspace, version, display, gpu, graphics, resolution, ram, cores, vcpus, ssh-port, root-patch, `nicN.enabled|mode|bridge|model|mac`) — `ConfigCmd::Set` — `config_commands`, `vm_commands`, `validate_names_the_field`
- [x] `config set --root-patch-reverted` (needed to turn root-patch mode off) — `ConfigCmd::Set::root_patch_reverted` — `config_commands` (`root-patch=maybe` refused); manual: `config set root-patch=off` without the flag is refused
- [x] `config recommend` — `ConfigCmd::Recommend`, `rg_core::Config::recommended` — `recommended_fits_the_host`
- [x] `workspace show` / `workspace set <dir>` — `WorkspaceCmd` — `vm_commands`, `preferences_and_workspace_root`
- [x] `workspace create` — `WorkspaceCmd::Create` on `rg_ops::Ops::prepare_workspace` — `prepare_creates_the_folder_and_extracts_boot_files`
- [x] `download [--version]` (recovery) — `Cmd::Download` on `rg_ops::Ops::download_media` — `recovery_download`, `prepare_and_download_wait_for_no_one_holding_the_lock`
- [x] `download --full` (full installer into the shared cache) — `Cmd::Download` on `rg_macos::download_installer` — `resume_continues_a_partial_download`, `installer_disc_and_cache`
- [x] `installer list` — `InstallerCmd::List` — `installers_and_gpus_list_with_headers`
- [x] `installer remove <build>` — `InstallerCmd::Remove` — `installers_and_gpus_list_with_headers` (`installer remove ../x` refused), `installer_disc_and_cache`
- [x] `disk create [--size]` (never overwrites, minimum 40) — `DiskCmd::Create`, `rg_core::Workspace::create_disk` — `create_disk_with_qemu_img`
- [x] `media remove --yes` — `MediaCmd::Remove`, `Workspace::remove_media` — `config_commands` (refused without `--yes`), `media_removal_and_disk_checks`
- [x] `vm list` (selected marked `*`) — `VmCmd::List` — `vm_commands`
- [x] `vm status [name]` — `VmCmd::Status`, `rg_qemu::running::find_running` — `vm_commands`, `running_detection`
- [x] `vm create <name> [--version] [--folder]` — `VmCmd::Create`, `rg_ops::Ops::create_vm` — `vm_commands`, `create_vm_adds_selects_and_prepares`
- [x] `vm select <name>` — `VmCmd::Select`, `Registry::select` — `preferences_and_workspace_root`; manual: `reims-gui vm select <name>` then `vm list` marks it
- [x] `vm delete <name> [--purge] [--yes]` (purge refused while running) — `VmCmd::Delete`, `rg_ops::Ops::delete_vm` — `vm_commands`, `export_delete_move_refuse_a_running_vm`
- [x] `vm export <name> [file] [--full]` — `VmCmd::Export`, `rg_core::export_vm`, `rg_ops::Ops::export_bundle` — `vm_commands`, `bundle_round_trip`
- [x] `vm import <file.json|file.tar> [--folder]` — `VmCmd::Import`, `Registry::import_settings` / `import_bundle` — `import_settings_renames_a_taken_name`, `reads_a_bundle_written_by_go`
- [x] `vm move <name> [folder]` — `VmCmd::Move`, `rg_ops::Ops::move_vm` — `vm_commands`, `move_vm_moves_only_the_apps_files`
- [x] `vm start [--display app|window] [-v] [--verbose-boot]` (Ctrl+C = power button, second = kill) — `VmCmd::Start`, `rg_ops::Running::wait_blocking` — `settings_and_kexts_before_start`, `start_runs_qemu_with_the_snapshot_and_releases_the_lock`; manual: Ctrl+C twice on a running `vm start`
- [x] `vm start --kexts-snapshot` (hidden, from the MCP server) — `VmCmd::Start::kexts_snapshot` — `kexts_snapshot_round_trips`
- [x] `gpu list` — `GpuCmd::List`, `rg_host::Host::gpus` — `installers_and_gpus_list_with_headers`, `gpus_are_listed_with_eligibility`
- [x] `gpu reserve <pci-address>` / `gpu release` — `GpuCmd`, `Host::reserve_gpu` / `release_gpu` — `reserve_and_release`, `reserve_script_rejects_injection`, `pci_ids_are_validated`
- [x] `settings show` / `settings reset --yes` — `SettingsCmd` — `settings_and_kexts_before_start`
- [x] `reims install` — `ReimsCmd::Install`, `rg_reims::Reims::install` — `install_checks_release_and_sha256_then_activates`
- [x] `reims build [--release]` — `ReimsCmd::Build`, `Reims::build` — `reims_commands`, `build_checks_the_release_before_docker`
- [x] `reims status` — `ReimsCmd::Status`, `Reims::installed` — `reims_commands`

## 2. MCP server (`internal/mcp/server.go` → `rg_agent::mcp`)

Same tools, JSON schemas, annotations and consent rules: `tools`, `consent_first`. Input goes over
QMP instead of a VNC connection (documented deviation in `crates/rg-agent/API.md`).

- [x] `vm_list` — `Server::vm_list` — `tools`
- [x] `vm_status` — `Server::vm_status` — `tools`
- [x] `vm_start` (downloads settings.yaml kexts and ReimsVgpu.kext first, spawns `reims-gui vm start --display app`) — `Server::vm_start` — `vm_start_downloads_kexts_first`
- [x] `vm_shutdown` — `Server::vm_shutdown`, `rg_ops::Ops::shutdown` — `shutdown_needs_a_running_vm`
- [x] `vm_force_stop` (`confirm`) — `Server::vm_force_stop`, `rg_ops::Ops::force_stop` — `consent_first`
- [x] `screen_capture` (`max_width`, default 1280) — `Server::screen_capture` (QMP `screendump`) — `scale_down_keeps_small_frames`
- [x] `send_keys` — `Server::send_keys`, `rg_qemu::qcode` — `parse_chord_names`, `modifiers_release_in_reverse`
- [x] `type_text` (US, ABNT2; refuses untypable characters) — `Server::type_text`, `rg_qemu::keys::type_keys` — `type_keys_by_layout`, `a_capital_holds_shift_around_the_key`
- [x] `pointer` — `Server::pointer`, `qcode::abs` — `abs_spans_the_tablet_and_gives_two_events`
- [x] `installer_list` — `Server::installer_list` — `tools`
- [x] `installer_download` (progress notifications, cancel) — `Server::installer_download` — `overall_progress_only_grows`
- [x] `installer_remove` (`confirm`) — `Server::installer_remove` — `consent_first`
- [x] `install_offline` (`unattended` needs `erase_disk` and `accept_license`) — `Server::install_offline`, `rg_ops::Ops::install_offline` — `consent_first`, `install_offline_needs_consent_a_running_vm_and_its_disc`
- [x] `host_check` — `Server::host_check` — `tools`
- [ ] Live: an agent drives a VM end to end — see task 5.2 below

## 3. GUI (`internal/gui` → `rg_ui`)

Visual checks: `crates/rg-ui/QA.md`; snapshots in `crates/rg-ui/tests/snapshots/`.

### Window, theme, preferences

- [x] VirtualBox layout: toolbar, VM list with state, tabs, status bar — `rg_ui::app::App::{toolbar, vm_list, tab_strip, status_bar}` — `snapshot_main_light`, `snapshot_main_dark`
- [x] Light, dark and system themes with the Go colors — `rg_ui::theme` — `light_palette_matches_the_go_app`, `text_reaches_aa_contrast_on_every_surface`
- [x] Rounded modals with depth — `rg_ui::dialogs`, `theme` — `modals_are_rounded_with_a_stronger_shadow_than_popups`
- [x] Preferences: Workspace, Theme, settings.yaml (Open, Reset to default with confirmation) — `dialogs::Prefs`, `RealBackend::{set_workspace_root, set_theme, open_settings_file, reset_settings}` — `snapshot_preferences_light`, `preferences_live_in_the_database_and_settings_yaml`
- [x] VMs started elsewhere (CLI, MCP, the Go app) shown as running — `real::scan_loop`, `rg_qemu::running::find_running` — `running_detection`; manual: start a VM with `macos-kvm-gui vm start`, the list shows it running
- [x] VM stop with a message opens a dialog (`VM: Stopped`) — `App::poll` (`Event::State`) — `tasks_and_starts_report_through_events`
- [x] Closing the window while a VM runs asks to shut it down first (Go: "Shut down the VM before closing the app") — `App::refuse_close` cancels the close (`ViewportCommand::CancelClose`) while any VM this app started runs — `closing_is_refused_while_a_vm_runs`; manual: close the window with a VM running
- [ ] Go blocks switching to another VM while the current one runs or a task is busy — `reims-gui` allows it on purpose (several VMs); manual: confirm switching while one VM runs keeps its state and log

### Menus

- [x] File › Preferences… — `Action::Preferences` — `snapshot_preferences_light`
- [x] File › Offline installers… (list, delete with confirmation) — `FileDialog::installers`, `Task::RemoveInstaller` — `snapshot_installers_light`, `snapshot_installers_dark`, `installers_delete_asks_first`
- [x] File › Import VM… (portal file chooser or typed path, `.json` or `.tar` with folder and progress) — `FileDialog::import`, `Task::Import` — `snapshot_import_light`, `import_refuses_a_missing_file`, `import_runs_the_task_and_selects_the_new_vm`
- [x] File › Export VM… (settings JSON or full bundle with progress) — `FileDialog::export`, `Task::Export` — `snapshot_export_dark`, `snapshot_export_progress_light`
- [x] Machine › New… — `Action::New` — `snapshot_wizard_light`
- [x] Machine › Settings… — `Action::Settings` — `snapshot_settings_display_light`
- [x] Machine › Move… — `FileDialog::move_vm`, `Task::Move` on `rg_ops::Ops::move_vm` — `snapshot_move_light`, `move_points_the_vm_to_the_new_folder`
- [x] Machine › Remove… (with "also delete files" and its confirmation) — `Modal::Remove`, `RealBackend::remove_vm` — `creates_lists_saves_and_removes_vms`
- [x] Machine › Start — `Action::Start { verbose: false }` — `toolbar_start_runs_the_vm_and_shows_the_screen_tab`
- [x] Machine › Start with verbose boot (debug) — `Action::Start { verbose: true }` — `install_verbose_gpu_matches_go` (boot-args); manual: the menu item boots with the log on screen
- [x] Machine › Shut down (ACPI power button, Sleep explanation) — `Action::Shutdown`, `SHUTDOWN_NOTE` — manual: the note appears and the VM gets the power button
- [x] Machine › Force stop (confirmation) — `Modal::confirm_force_stop`, `Action::ForceStopNow` — `escape_closes_the_top_dialog`; manual: confirm, QEMU exits
- [x] Machine › Show in a window / Full screen — the embedded window is released to the desktop (VNC: own viewport) and comes back; Full screen slides the shell away — `show_in_a_window_detaches_and_comes_back`, `full_screen_hides_the_shell_and_comes_back`; live: task 5.2
- [x] Help › Online Documentation, Web Site, Bug Tracker — `Action::Url` — manual: each opens the browser
- [x] Help › About (version, boot files) — `Action::About`, `RealBackend::about` — manual: shows the version and `asset_versions`

### Toolbar

- [x] New, Import, Settings, Remove, Start, Shut down, Force stop, Window, Full screen with the Go enable rules — `App::toolbar`, `App::enabled` — `snapshot_main_light`, `toolbar_start_runs_the_vm_and_shows_the_screen_tab` (Import, Window and Full screen act through the pending items above)
- [x] State chip at the right — `App::state_chip` — `snapshot_main_light`
- [x] Shortcuts (Ctrl+N, I, E, S, comma, Enter, H, Delete, F11) — `app::keys`, `App::shortcuts` — manual: QA.md "States"

### Tabs

- [x] Details: General, System, Display, Storage, Audio, Network, USB, GPU sections — `tabs::details` — `snapshot_main_light`
- [x] Details: Preview over the VNC socket — `RealBackend::preview`, `rg_qemu::vnc` — `preview_rects_are_cut_from_the_frame`, `capture_waits_for_pixels_then_quiet`
- [x] Screen: Reims vGPU window embedded by X11 reparenting (design D3) — `rg_ui::screen` (`embed.rs`) on `rg_x11::Embedder`, found by pid off the UI thread — `sdl3_reims_window`, `release_puts_child_back_on_root`, `snapshot_screen_embedding_light`, `snapshot_screen_not_found_dark`, test target `xvfb_embed` (a real SDL window inside the real eframe window); live: task 5.2
- [x] Screen: VMware SVGA embeds QEMU's GTK window over X11 (`ScreenSource::Window { reims: false }`), VNC without X11 (`ScreenSource::Vnc`) — `display_embeds_qemu_gtk_for_svga_and_reims_window_over_x11`, `qemu_window_keeps_vnc_for_svga_only`; live 2026-10-16: Nilton (Sequoia 15.8.1) on SVGA embedded, login typed in the tab, desktop shown, shutdown over SSH detected. QEMU's SDL display was dropped (withdraws its window without text consoles)
- [x] Screen: X11/XWayland backend choice at startup — `screen::backend_choice::{x11_available, native_options}` used by `run.rs` — `no_x_server`; live on GNOME Wayland: task 5.2b
- [x] Screen: VNC texture fallback without X11, with keyboard and pointer (Go `screen.go`) — `screen/vnc.rs` (dirty rects, letterbox, Ctrl+Alt grab, absolute pointer) — `snapshot_screen_vnc_dark`, `vnc_screen_grabs_the_keyboard_on_click`, `ctrl_alt_releases_every_key`, `keys_map_by_position`
- [x] Screen: overlay rule (rectangle reported, hidden under dialogs and menus) — `ScreenArea`, `Backend::set_screen_area` — `screen_area_is_reported_and_hidden_under_dialogs` (the embedder hides QEMU's window under them: `screen::embed`)
- [x] Install: 5-step stepper (VM folder, recovery, disk, install, clean up) with progress — `tabs::install`, `Task::{PrepareFolder, DownloadRecovery, CreateDisk, RemoveMedia}` — `snapshot_install_light`, `tasks_and_starts_report_through_events`
- [x] Install: disk size field (Go: editable, default 80, at least 40) and the "Disk created with a warning" dialog — `tabs::disk_task`, `Event::Notice` from `Task::{CreateDisk, CreateVmFiles}` — `disk_size_must_be_a_whole_number`; the 40 GB minimum is `rg_core::Workspace::create_disk`'s
- [x] Install: Download full installer / Delete installer (confirmation) — `Task::{DownloadInstaller, DeleteInstaller}` — `snapshot_install_light`
- [x] Install: "Install from the full installer" box (Unattended check, command, Copy, Type it in the VM, then the Screen tab), shown while a VM started here has the OFFLINE disc — `Backend::installer_command`, `Task::InstallOffline` on `rg_ops::Ops::install_offline` — `offline_install_box_types_the_line_and_shows_the_screen`, `offline_install_box_is_hidden_without_the_disc`; live: an offline install on the test machine
- [x] Install: Remove install media confirmation then "N GB freed" dialog; Settings › Storage "Remove…" — `Task::RemoveMedia` result as a dialog — `removing_install_media_says_how_much_was_freed`, `storage_remove_asks_then_removes_the_media`
- [x] Install: Start the VM / Start with verbose boot — `tabs::install` — `snapshot_install_light`
- [x] Host: checks with Check again and Set up host — `tabs::host`, `Task::SetupHost` — `snapshot_host_light`, `snapshot_host_dark`, `host_facts_load_in_the_background`
- [x] Host: Reims vGPU section (status, Install Reims vGPU, Build from source in Docker with confirmation) — `tabs::host`, `Task::{InstallReims, BuildReims}` — `snapshot_host_light`
- [x] GPU: adapters with eligibility and reason, Reserve for VFIO (confirmation), Release reservation — `tabs::gpu`, `Task::{ReserveGpu, ReleaseGpu}` — `snapshot_gpu_light`
- [x] GPU: Root-patch mode switch with Go's two confirmations (turn on: SIP 0x803 and kexts, OCLP's kexts downloaded now; turn off: "did you revert the root patch in OCLP?"); saved only once confirmed — `Task::RootPatch` — `root_patch_mode_changes_only_once_confirmed`
- [x] Log: the task and QEMU output — `tabs::log_tab` — `tasks_and_starts_report_through_events`

### Dialogs

- [x] Settings › General (name, OS, folder) — `dialogs::Settings::general` — `snapshot_settings_display_light`
- [x] Settings › General › Move… button (closes Settings, opens Move, as Go) — `Settings::general`, `Action::Move` — `settings_move_opens_the_move_dialog`
- [x] Settings › System (RAM, CPUs, threads, Recommended for this host) — `Settings::system` — `cores_must_be_a_power_of_two`, manual: QA.md
- [x] Settings › Display (Shown, Graphics controller, Reims resolution, wide-resolution warning, METAL_ERROR_MODE commands with Copy) — `Settings::display`, `reims_box` — `snapshot_settings_display_light`, `snapshot_settings_display_dark`, `settings_ok_saves_the_new_resolution`
- [x] Settings › Storage, Audio, USB (read-only facts) — `Settings::ui` — manual: QA.md
- [x] Settings › Network (Adapter 1–4, NAT/bridged, bridge, model, MAC with regenerate, driver warnings) — `Settings::network`, `nic_form` — `nic_driver_matrix`, `validate_nics_refuses_bad_adapters`
- [x] New VM wizard (identity, hardware, network, disk; Reims by default for Sequoia/Tahoe when installed) — `dialogs::Wizard`, `Task::CreateVmFiles` — `snapshot_wizard_light`, `snapshot_wizard_dark`, `wizard_defaults_sequoia_to_reims_and_refuses_an_empty_name`
- [x] Confirmations and error dialogs; Esc and Enter — `Modal::{Confirm, Error, Info}` — `escape_closes_the_top_dialog`
- [x] Import, Export (with progress), Move, Offline installers dialogs — see Menus

## 4. Core behavior (`internal/core` → crates)

### Data and compatibility

- [x] SQLite registry, schema 1–5 migrations, legacy file import — `rg_core::registry::Registry` — `reads_the_go_database`, `round_trips_the_go_database`, `migration_from_version_1`, `legacy_files_are_imported_once`
- [x] Config JSON as Go writes it; validation messages — `rg_core::config::Config` — `json_matches_go`, `missing_json_fields_take_the_defaults`, `validate_names_the_field`
- [x] Legacy (null) vs empty network lists — `rg_core::network` — `legacy_and_empty_networks_survive_the_database`, `legacy_vm_keeps_its_network`
- [x] Workspace flock compatible with Go (`lock_path`) — `rg_core::workspace::Workspace::lock` — `lock_path_matches_go`, `lock_is_exclusive_and_outside_the_workspace`
- [x] Run directory `macos-kvm-gui-*` with `qmp.sock`, `qmp-control.sock`, `vnc.sock` — `rg_qemu::vm` — `run_dir_is_private_and_named_for_both_apps`, `check_run_dir_accepts_only_private_dirs`
- [x] Running-VM detection through `/proc` (either app, OSX-KVM's relative paths) — `rg_qemu::running::find_running` — `running_detection`, `finds_osx_kvm_relative_quoted_path`
- [x] settings.yaml (`theme`, `assets_dir`, `reims_qemu`, extra kexts), env > settings.yaml > `.env` — `rg_core::settings` — `settings_yaml_over_env_file`, `env_file_values`, `extra_kexts`
- [x] Preferences in the DB (selected VM, Workspace root, theme) — `Registry::{preference, set_preference}` — `preferences_and_workspace_root`

### Boot files

- [x] Embedded OpenCore, kexts, OVMF, `MKG_ASSETS_DIR` override — `rg_assets::BootAssets` — `embedded_set_has_the_boot_files`, `assets_dir_override_is_reported_and_checked`
- [x] `config.plist` byte-identical to Go — `rg_assets::opencore_config` — golden fixtures `crates/rg-assets/tests/fixtures/*.plist`: `default_matches_go`, `high_sierra_matches_go`, `mojave_install_matches_go`, `tahoe_reims_with_extras_and_switches_matches_go`
- [x] SMBIOS identity, NVRAM, EFI extraction, OpenCore volume hidden — `rg_assets::prepare_boot`, `Identity` — `identity_has_go_shapes`, `prepare_extracts_boot_files_once`, `prepare_hides_opencore_volume`
- [x] Verbose boot (`-v debug=0x100`, one start) — `BootOptions::verbose_boot` — `install_verbose_gpu_matches_go`

### Install media

- [x] Recovery download (Apple's osrecovery, fixed names, nothing left on failure) — `rg_macos::download_recovery` — `recovery_download`, `recovery_interrupted_leaves_nothing`, `recovery_ignores_server_file_names`
- [x] Chunklist RSA-2048 signature and per-chunk SHA-256 — `rg_macos::verify_image` — `real_chunklist_signature`, `verify_image_cases`, `chunklist_huge_count_does_not_panic`
- [x] DMG conversion with `qemu-img`, previous media kept on failure — `rg_macos::download_media` — `convert_progress`, `download_failures_preserve_media`
- [x] Full installers: catalog, board ID filter, resumable download, integrity, shared cache — `rg_macos::{find_installers, download_installer}` — `catalog_parsing`, `resume_continues_a_partial_download`, `resume_restarts_a_file_longer_than_the_package`, `info_json_matches_go`
- [x] Offline disc (`OFFLINE`, install script byte-identical), attached with the recovery, shared lease — `rg_macos::use_disc`, `rg_qemu::args` — `installer_disc_and_cache`, `installer_disc_rides_along_with_the_recovery`
- [x] Offline install typing (Terminal chords, `-u` unattended, layout check) — `rg_ops::Ops::install_offline`, `rg_qemu::keys::recovery_terminal` — `install_line_adds_unattended`, `recovery_terminal_starts_with_control_f2`, `guest_keys_refuse_layout_dependent_letters`

### Disks

- [x] Thin qcow2 disk, never overwritten, size warning — `rg_core::Workspace::create_disk` — `create_disk_with_qemu_img`
- [x] Remove install media (freed bytes) — `Workspace::remove_media` — `media_removal_and_disk_checks`
- [x] Virtual size of a qcow2, standalone-disk check for bundles — `rg_core::network::disk_virtual_size`, `bundle::check_standalone_disk` — `qcow2_virtual_size`, `bundle_rejects_a_disk_with_a_backing_file`

### QEMU and VM lifecycle

- [x] QEMU arguments, VMware SVGA — `rg_qemu::args::build_args` — golden `crates/rg-qemu/testdata/buildargs-svga.golden`, `svga_matches_go`, `build_args_per_version`
- [x] QEMU arguments, Reims vGPU (`apple-gfx-pci`, memfd, audio) — `build_args` — golden `buildargs-reims.golden`, `reims_matches_go`, `reims_args_rewrite_the_common_ones`
- [x] Start pipeline (validate, kexts, disk, running, bridges, SSH port, GPU, Reims checks, lock, boot files, disc) — `rg_ops::Ops::start_vm` — `start_refuses_invalid_missing_and_running`, `start_refuses_a_bad_settings_yaml_and_a_missing_bridge`, `start_refuses_the_ssh_port_in_use_and_a_locked_workspace`, `start_runs_qemu_with_the_snapshot_and_releases_the_lock`
- [x] QEMU exit explained (KVM busy, disk locked, SSH port, bridge, udmabuf, /dev/kvm) — `rg_qemu::vm::explain_exit` — `explain_exit_messages`, `exit_status_reads_like_go`, `udmabuf_exit_is_explained`
- [x] QEMU never outlives the app (`setpriv --pdeathsig`) — `rg_qemu::vm::Vm::start` — `start_runs_and_reports_the_exit`; manual: kill the app, QEMU exits
- [x] QMP client, shutdown (power button) and force stop (`quit`), pause/resume events — `rg_qemu::qmp::Qmp`, `rg_ops::Ops::{shutdown, force_stop}` — `qmp_commands_events_errors_timeout_and_disconnect`, `qmp_events_pause_and_resume`, `shutdown_needs_a_running_vm`
- [x] RFB client (Raw, CopyRect, DesktopSize, extended keys) — `rg_qemu::vnc::Vnc` — `handshake_updates_resize_and_input`, `plain_key_events_without_ext_keys_and_no_auth`, `bgrx_to_rgba_swaps_and_is_fast_at_4k`
- [x] Memlock limit check for passthrough — `rg_qemu::vm::memlock_limit` — `memlock_limit_reads_proc`

### Network adapters

- [x] Up to 4 adapters, NAT/bridged, models per version, MACs, SSH forward on adapter 1 — `rg_core::network` — `net_args_for_four_adapters`, `nic_driver_matrix`, `net_args_place_and_forward`
- [x] Bridge helper ACL and preflight, SSH port in use — `check_bridges`, `check_ssh_port` — `bridge_acl_and_preflight`, `ssh_port_in_use`

### GPU passthrough

- [x] Listing and eligibility (Dortania table, Kepler, boot display, IOMMU groups, USB functions, identical cards) — `rg_host::Host::gpus` — `gpus_are_listed_with_eligibility`, `compat_table_follows_dortania`, `boot_display_is_refused`, `shared_group_names_the_foreign_device`, `iommu_off_is_refused`, `identical_second_card_is_refused`
- [x] VFIO reservation and release (pkexec, modprobe.d, udev) — `Host::reserve_gpu`, `release_gpu`, `reserved_ids` — `reserve_and_release`, `reserve_script_rejects_injection`
- [x] Ready check before QEMU (driver, memlock, version, vfio node, patch kexts) — `Host::gpu_ready` — `gpu_ready_checks_driver_memlock_version_and_vfio_node`, `kepler_gpu_follows_the_macos_version`, `patch_kext_gpu_needs_a_kext_in_settings_yaml`
- [ ] Live: passthrough of a real GPU — untested on real hardware in both apps; manual: reserve, reboot, start a Monterey VM with an eligible card

### Root-patch mode

- [x] OCLP 2.5.1 kexts with pinned SHA-256, config on/off/clear, marker cleared after 60 s — `rg_assets::{download_root_patch_kexts, clear_root_patch_marker}` — `root_patch_kexts_download_and_transitions`, `root_patch_config_on_off_and_clear`, `root_patch_on_matches_go`, `root_patch_clear_matches_go`, `unpack_refuses_escapes_and_size_mismatches`
- [x] Refused without cached kexts — `prepare_boot` — `root_patch_without_cached_kexts_is_refused`
- [x] GUI confirmations — see GPU tab above

### Extra kexts (settings.yaml)

- [x] Download with SHA-256, archive checks, boot order, removal, clashes, kext switches — `rg_assets::{download_extra_kexts, kext_switches}` — `check_kext_archive_accepts_and_orders`, `download_checks_hash_and_downloads_once`, `extra_kexts_boot_order_removal_and_clashes`, `kext_switches_disable_built_in_kexts`
- [x] Snapshot per start (CLI, MCP) — `rg_ops::Ops::start_vm` — `start_runs_qemu_with_the_snapshot_and_releases_the_lock`, `vm_start_downloads_kexts_first`

### Reims vGPU

- [x] Packages: manifest, HTTPS download capped, SHA-256, safe unpack, activation, pruning — `rg_reims::Reims::install`, `activate` — `install_checks_release_and_sha256_then_activates`, `unpack_refuses_unsafe_archives`, `unpack_enforces_the_expansion_budget`, `activate_switches_current_stages_other_releases_and_prunes`, `in_use_sees_processes_inside_a_build`
- [x] Docker build from the embedded Dockerfile — `Reims::build` — `build_checks_the_release_before_docker`, `manifest_matches_the_dockerfile_commit`, `dockerfile_chowns_on_exit`; manual: `reims-gui reims build` on this host
- [x] Which QEMU runs (setting, Workspace, reims-ctl) — `rg_core::settings::find_reims_qemu`, `Reims::qemu` — `qemu_prefers_the_setting_then_the_workspace_then_reims_ctl`, `reims_qemu_setting_then_install_folders`
- [x] Host checks (QEMU with `apple-gfx-pci`, Vulkan, `vm.max_map_count`, udmabuf) — `rg_host::Host::reims_checks` — `reims_checks_cover_qemu_vulkan_map_count_and_udmabuf`, `map_count_formula`, `missing_udmabuf_warns_to_load_the_module`
- [x] ReimsVgpu.kext for Tahoe (pinned SHA-256) — `rg_assets::download_reims_kext` — `reims_kext_download_and_boot`
- [x] Resolutions and the wide-resolution rule — `rg_core::config::{RESOLUTIONS, wide_resolution}` — `wide_resolutions`, `validate_graphics_resolution_gpu_root_patch`
- [x] Reims-first default for Sequoia and Tahoe — `dialogs::Wizard::new` — `wizard_defaults_sequoia_to_reims_and_refuses_an_empty_name`

### Host

- [x] Checks (virtualization, /dev/kvm, QEMU ≥ 8.2.2, AVX2 per version, ignore_msrs, other hypervisors) — `rg_host::Host::checks_for` — `checks_ready_host_are_all_ok`, `checks_report_problems`, `qemu_version_is_parsed`, `old_version_needs_no_avx2_and_missing_proc_is_inconclusive`
- [x] Setup (packages, kvm module and group, ignore_msrs, max_map_count) and fix ignore_msrs — `Host::setup_host`, `fix_ignore_msrs` — `setup_host_runs_the_script_with_checked_arguments`, `fix_ignore_msrs_reports_each_outcome`

### Export, import, move

- [x] Settings JSON export/import (taken names renamed) — `rg_core::export_vm`, `Registry::import_settings` — `import_settings_renames_a_taken_name`
- [x] `.tar` bundle (disk, NVRAM, identity, media), Go bundles readable — `rg_core::bundle` — `bundle_round_trip`, `reads_a_bundle_written_by_go`, `bundle_rejects_unexpected_entries`
- [x] Move only the app's files; refused while running — `Registry::move_vm`, `rg_ops::Ops::move_vm` — `move_vm_moves_only_the_apps_files`, `export_delete_move_refuse_a_running_vm`

### Keyboard layouts

- [x] Host layout → recovery keyboard (`prev-lang:kbd`), ABNT2 — `rg_core::keyboard` — `keyboard_layout_ids`, `parses_debian_and_localectl`, `recovery_keyboard_goes_to_prev_lang_and_nvram_delete`
- [x] Physical keys from X11 keycodes and evdev (ISO, ABNT2, keypad) — `rg_qemu::keys` — `x11_keycodes_are_physical_keys`, `key_table`
- [ ] Live: ABNT2 keys, keypad Enter and Divide reach macOS through the embedded window — task 5.2

## 5. Build and packages

- [x] Docker-only build for 24.04 and 26.04 — `scripts/cargo-docker.sh`, `scripts/build.sh`, `docker/Dockerfile` — manual: `scripts/build.sh 24.04` and `26.04` leave the binary, `.tar.gz`, `.deb` and `.sha256` files in `dist/`
- [x] `.deb` installs in a clean container and renders under Xvfb — `scripts/check-package.sh` — manual: `scripts/check-package.sh 24.04` and `26.04` print `ok`
- [x] License texts and notices in the packages, source archive — `packaging/package.sh`, `packaging/third-party-licenses.py`, `scripts/source-archive.sh` — `check-package.sh` (checks `THIRD_PARTY_LICENSES`)

## 6. Live tests on this host (task 5.2)

Unchecked until run and recorded here (date, result). Host: this X11 KDE machine, the Nilton VM.

- [x] **Reims embedded.** Select the Nilton VM on Reims vGPU, Start. Expect: no separate "Reims vGPU" window on the desktop; the Screen tab shows macOS; `xwininfo -root -tree | grep -i "reims vgpu"` lists the window as a child of the app's window. (2026-10-16, 0.1.1 `.deb`: embedded, login typed in the tab, Preview in Details live, Help menu hides the guest.)
- [x] **SVGA in-app screen.** Set the Nilton VM to VMware SVGA, Display: In the app, Start. Expect: QEMU's GTK window inside the Screen tab, typing works; the Preview in Details updates. (2026-10-16, this host: embed and typing passed; 2026-10-17, 0.1.1: Preview in Details live.)
- [x] **Overlay.** With the VM running and the Screen tab shown, open File (menu), Machine › Settings… is disabled, open Help › About, and a combo popup in Preferences. Expect: the guest disappears while each is open and comes back when closed; clicks on the dialog never reach macOS (watch the pointer in the guest). (2026-10-17, Reims: Help menu and Preferences with its Theme popup hide the guest (window unmapped), it comes back on close, a click over its place while hidden left macOS untouched; the File menu, which does not cover the tab, leaves it shown. Found and fixed: a short dialog kept the height of the last tall one (footer filled the space), and with the app idle a click whose press and release land in one frame opened a combo's popup under its own dialog.)
- [x] **Mode change.** In macOS, System Settings › Displays, pick another resolution (Reims: also change it in Settings › Display after a stop). Expect: the guest stays the size of the tab, no window outside the app. (2026-10-17, Reims: 1920x1080 → 1024x768 and back through CoreGraphics in the guest; the window stayed 897x624 in the tab, macOS letterboxed.)
- [x] **Resize.** Resize the app window and toggle the Log tab and back. Expect: the guest follows the tab; switching tab hides it. (2026-10-17: 1280x820 → 1100x700, the guest went 897x624 → 737x504; Log unmaps it, Screen maps it again.)
- [x] **Restart.** Shut down macOS from the Apple menu, Start again. Expect: the Screen tab gets the new window; no stale window. (2026-10-16, 0.1.1: shut down over SSH, Start again, re-embedded.)
- [x] **Keyboard (ABNT2).** Host layout `br(abnt2)`, macOS layout Brazilian ABNT2. Click the screen, type `ç ´ ~ / ?` with the ABNT2 keys, keypad Enter and keypad `/` in TextEdit. Expect: each key arrives as itself. Ctrl+Alt+Super releases the keyboard (Reims). (2026-10-17, X keycodes through XTEST, Reims and SVGA alike, text read back from TextEdit: `ç`, `´`+e = `é`, `~`+a = `ã`, keypad Enter, keypad `/`, Option+Q `/`, Option+W `?` arrive. The ABNT2 `/?` key (USB International1, delivered to the guest's keyboard) types nothing: macOS's Brazilian ABNT2 layout gives that key nothing, as with an ABNT2 USB keyboard on a Mac; documented.)
- [x] **Frame rate.** Run the VM standalone (`reims-gui vm start --display window`) and embedded, each with `REIMS_VGPU_STATS=1` in the environment, playing the same animation for 60 s. Expect: embedded ≥ 90% of the standalone frames per second (record both numbers). (2026-10-17, RTX 4070 host, 897x624 both: a requestAnimationFrame canvas in Safari, 73.7/s embedded and 73.7/s standalone (100%); the 30 fps aerial screensaver, 29.9/s both. The embedded numbers come from `RG_DEBUG=1`, which now echoes QEMU's output on stderr. Found and fixed: the app embedded the Reims window of a VM started outside it (`vm start --display window`); only VMs started here are embedded now.)
- [x] **Both apps at once.** Start the VM from `macos-kvm-gui`, open `reims-gui`. Expect: it shows the VM running, Shut down works over the control socket, Start is refused ("already running" / disk in use). (2026-10-17, Go app at main 75240bf, `vm start`: `reims-gui vm start` refused with the disk-in-use message; the GUI shows "Running (outside the app)" and leaves the Reims window alone; Shut down sent the power button, Force stop stopped the Go app's QEMU through its control socket and the Go CLI exited. Found and fixed: outside VMs showed as plain Running with the embedded-display hints.)
- [x] **CLI.** `reims-gui vm start --vm Nilton` then Ctrl+C (power button) and Ctrl+C again (kill). Expect: the Go messages, exit status 0 or 1 as Go. (2026-10-17: "asking macOS to shut down (Ctrl+C again to force)", QMP POWERDOWN, then "forcing the stop", "Stopped by the user (forced).", QEMU gone; same code path as Go's cmd/vm.go.)
- [x] **MCP.** `claude mcp add reims-gui -- reims-gui mcp`; ask the agent to start the VM, take a screenshot, type in Spotlight and shut it down. Expect: each tool succeeds; the app shows the VM running. (2026-10-17, a stdio MCP client: vm_start, vm_status, screen_capture (1920x1080 scaled to 1280), pointer click, type_text (password, `TextEdit` in Spotlight), send_keys, vm_shutdown all succeeded; the GUI showed the VM "Running (outside the app)". macOS logged in ignores the power button, as the tool says.)
- [x] **Screenshots.** Capture `docs/screenshots/reims-gui-*.png` (main, Screen with a running Reims VM, Install, Host, Settings, wizard; light and dark). (2026-10-17: Install, Host, GPU, New VM, Host and New VM dark added to the earlier Screen, Details, Settings and dialog shots.)

## 7. Release gate: GNOME Wayland (task 5.2b, design D3 risks)

### Automated: GNOME's compositor in a container (`scripts/check-gnome.sh <release>`)

Mutter headless (Wayland + XWayland, the compositor of GNOME's session) in a clean
`ubuntu:<release>` container with the release `.deb` and `/dev/kvm`; no GPU (lavapipe), so the
Reims window is stood in by an SDL3 window titled like it.

- [x] 24.04 (Mutter 46.2, Xwayland 23.2) / [x] 26.04 (Mutter 50.1, Xwayland 24.1.10), 0.1.1, 2026-10-16: `--self-test` on XWayland and on native Wayland; a VMware SVGA VM started from the GUI on XWayland gets QEMU's GTK window embedded in the Screen tab (window tree: app › container › QEMU, inside Mutter's frame); SIGTERM to the GUI puts the window back first and Xwayland survives.
- [x] Both releases, same date: an SDL3 window titled `Reims vGPU (port 2)` found by PID and embedded into another X11 window under Mutter, alive for 10 s (`rg-x11` examples `sdl_window` and `find_window`).
- Found and fixed by these runs: Mutter (XWayland) lets go of a framed window asynchronously and took QEMU's window back out of the container; `embed` now waits for the window to return to the root before reparenting. Xwayland 24.1.10 crashes (SIGSEGV) when a client dies with a foreign window in its save-set inside a Mutter frame; the app now releases embedded windows on SIGTERM, SIGINT and SIGHUP. A SIGKILL or crash of the app with a VM on screen still hits that Xwayland bug on 26.04 (upstream).

### Live: GNOME Shell with the GPU and the VM (2026-10-17)

A real GNOME Shell (`gnome-shell --headless --wayland`, a 1920x1080 virtual monitor, XWayland,
systemd/logind and PipeWire) in an `ubuntu:24.04` (GNOME Shell 46.0) and an `ubuntu:26.04` (GNOME
Shell 50.1) container with `/dev/kvm`, `/dev/dri` and `/dev/udmabuf`, the 0.1.1 `.deb` (plus the
fixes below), Reims vGPU installed from the release package (`reims-gui reims install`), all host
checks OK, and the Nilton VM booting from a qcow2 overlay over this host's disk (read-only). Input
came through Mutter's RemoteDesktop API, as a real mouse and keyboard; screenshots through its
ScreenCast API. Reims vGPU rendered on the RX 6700 XT (radv).

- [x] 24.04 / [x] 26.04 **Session, embedding.** The app runs on XWayland in GNOME's frame; Start puts the Reims window inside the Screen tab (window tree: app › container › Reims vGPU), none on the desktop.
- [x] 24.04 / [x] 26.04 **Typing in the guest.** Click on the guest, type the password, Enter: logged in (`who` shows the console session); text typed into TextEdit read back (`abC 1!`, `AbC Xy`). Found and fixed: Mutter answers a click on the embedded window by focusing the app's toplevel, which took the keyboard back from QEMU (keys went to egui); `rg-x11` now watches FocusIn on the app's window and hands the keyboard to QEMU when the pointer is over it (test `enter_gives_focus_only_while_the_app_is_active`).
- [x] 24.04 **Overlay hiding.** Help menu: the guest window unmapped ("hidden while a dialog or menu is open"), back on close.
- [x] 24.04 **Guest mode change.** 1920x1080 → 1024x768 → 1920x1080 in macOS: the embedded window stayed 952x652.
- [x] 24.04 / [x] 26.04 **Keyboard capture and release.** With GNOME's `org.gnome.mutter.wayland xwayland-allow-grabs` on and the app's class in `xwayland-grab-access-rules`, Mutter logs "XWayland window (reims-gui) has a grab granted" when the guest captures, and Super and Alt+F2 reach macOS (no overview, no Run dialog), the first capture after a VM start included; Ctrl+Alt+Super releases. With GNOME's defaults Super opens the overview (Mutter ignores XWayland grabs), the same for Reims vGPU's own window. Now: a host check "GNOME shortcuts in the guest" (Warning when off, GNOME Wayland sessions only) and Set up host turns it on for the user with gsettings, keeping other rules, before the password prompt (tested live: `reims-gui setup` applied it, the check went to OK, Super stayed in the guest). Measured first with plain X11 windows: a toplevel, a child, and a foreign window reparented into another client's (as the embedding does) all get the grab granted, on GNOME 46 and 50. 24.04 (0.1.3): Warning → `reims-gui setup` → OK; 6 of 6 captures (app focused or not) kept Alt+F2 in the guest. Known: in some sequences Mutter stops honoring a grab SDL still holds (seen after a GNOME dialog or the overview had the focus, and once after the macOS login); Ctrl+Alt+Super and a click on the guest bring it back. It cannot be forced from the app: taking QEMU's focus away makes Reims vGPU release the capture, which only a click takes again. The guest's resolution change is not the cause (a capture held across one stayed honored).
- [ ] 26.04 **Shortcuts in Reims vGPU's own native Wayland window** (the no-X11 fallback only), with xdg-permission-store running as in a logged-in session: captured, Alt+F2 still opened GNOME's Run dialog and GNOME showed no inhibit prompt; Super alone did not tell the states apart. That window and its keyboard-shortcuts-inhibit request are Reims vGPU's (SDL3), not reims-gui's; the embedded path (the default with XWayland) uses the X11 grab above. To report upstream.
- [x] 26.04 **VM restart.** Shut down over SSH, Start again in the same session: re-embedded, captured, shortcuts in the guest.
- [x] 26.04 **HiDPI and fractional scaling.** `gdctl set ... --scale 2` and `--scale 1.25` with the guest embedded: the container stays exactly the tab's rectangle and the guest fills it, no offset. With two monitors (100% and 200%, XWayland native scaling, root 5760x2160) and gsd-xsettings publishing Xft.dpi 192, as in a logged-in session, the app's UI comes out at its size (2 pixels per point) and the container is the tab's rectangle × 2 (3184x1686 at 648,272).
- [x] 26.04 **Monitor move.** Super+Shift+→ moved the app from the 100% to the 200% monitor: the guest stays embedded, the container follows the window's new size (3168x1696), sharp at 2x.
- [x] 26.04 **Fallback.** `env -u DISPLAY reims-gui`: the app runs as a native Wayland window, Start opens Reims vGPU's own window (native Wayland), and the Screen tab says "Reims vGPU opened its own window". Found and fixed: Details said "Screen tab (… embedded)" and the status bar "click it to type" in that case.
- [ ] 24.04 / [ ] 26.04 **Frame rate.** On the RX 6700 XT (radv) the guest itself presents about 6 frames a second with the aerial screensaver (5.3/s embedded, 6.4/s in its own window on 24.04, one 60 s sample each): Reims vGPU's rendering on radv is the limit (guest presents = window presents in both). The ≥ 90% gate is met on the RTX 4070 host (section 6: 73.7 vs 73.7). Safari did not respond in the guest on radv.
- Found: SDL's XWayland pointer lock crashed Xwayland 24.1.10 (26.04) once ("lock_pointer: null value passed for arg 2"), on an XTEST click into the captured guest; not reproduced with real (RemoteDesktop) input. Reims vGPU/SDL3 upstream.

### Manual: real GNOME sessions

Run on a fresh install of **Ubuntu 24.04** and again on **Ubuntu 26.04**, logged in to the default
GNOME session (Wayland), with the release `.deb` installed (`sudo apt install ./reims-gui_*.deb`)
and Reims vGPU installed from the Host tab. Record date, release, GPU and result per line.

- [ ] 24.04 / [ ] 26.04 **Session.** `echo $XDG_SESSION_TYPE` prints `wayland` and `echo $DISPLAY` is set (XWayland). Start `reims-gui` from the app grid. Expect: it opens; `xwininfo -root -tree | grep reims-gui` finds it (it runs on XWayland).
- [ ] 24.04 / [ ] 26.04 **Embedding.** Start a Reims VM. Expect: the guest inside the Screen tab, no standalone window in the Activities overview.
- [ ] 24.04 / [ ] 26.04 **Keyboard capture and release.** Click the guest, type; press Super. Expect: Super goes to macOS, not GNOME's overview, while captured; Ctrl+Alt+Super releases and Super opens the overview again. Alt+Tab while released switches apps.
- [ ] 24.04 / [ ] 26.04 **Guest mode change.** Change the macOS resolution. Expect: the guest stays the tab's size.
- [ ] 24.04 / [ ] 26.04 **Overlay hiding.** Open a menu, a dialog and a combo popup over the Screen tab. Expect: the guest hides until they close.
- [ ] 24.04 / [ ] 26.04 **VM restart.** Shut down and start again. Expect: re-embedded.
- [ ] 24.04 / [ ] 26.04 **HiDPI and fractional scaling.** Settings › Displays › Scale 200%, then 125% (fractional scaling on). Expect: the guest fills exactly the tab, no offset or blur at the edges.
- [ ] 24.04 / [ ] 26.04 **Monitor move.** With two monitors of different scale, drag the app from one to the other. Expect: the guest follows and keeps the tab size.
- [ ] 24.04 / [ ] 26.04 **Fallback.** Log in with `DISPLAY` unset for the app (`env -u DISPLAY reims-gui`). Expect: the app runs on Wayland, the Reims window opens on its own and the Screen tab says so.
- [ ] 24.04 / [ ] 26.04 **Frame rate.** As in section 6, ≥ 90% of the standalone window.

### GNOME recheck (2026-10-17, GNOME Shell 46 headless, rg-live-img:24.04)

- [ ] **Super in the guest, retested: fails in both apps.** With `xwayland-allow-grabs` and the app's class in the rules, Mutter logs "has a grab granted" for the app's window, but once rg-x11 moves the X focus to QEMU's child Mutter's focus window becomes NULL ("Focus change has no effect, because there is no matching wayland surface") and Super opens the overview. reims-gui 0.1.4 and the Go app behave the same. This contradicts the earlier "6 of 6 captures kept Alt+F2" result above: the harness differed (here a single long-lived RemoteDesktop session, with Mutter VERBOSE logging), so the two need reconciling on a logged-in GNOME session before the shortcut behavior is claimed. Direction: keep Mutter's focus on the app window (focus the container's parent and forward keys, or XEmbed-like focus) instead of focusing QEMU's child.

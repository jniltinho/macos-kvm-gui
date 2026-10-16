# rg-core public API

Layer 0: types, persistence and locks only; nothing here calls a higher crate. Errors are
`rg_core::Error` (`Field(FieldError)`, `Busy`, `PortInUse(u16)`, `Io`, `Sql`, `Json`, `Msg`);
`Result<T>` is `Result<T, Error>`. Shut-down checks (Go's `FindRunning`) are the caller's job.

- `config`: `Config` (Go JSON; `extras`, `verbose_boot` never serialized), `Config::defaults()`,
  `.recommended(threads, mem_mib)`, `.validate()`, `.workspace_dir()`, `.reims()`, `.reims_kext()`,
  `.in_app()`, `.reims_resolution()`; `VERSIONS`, `version_index`, `version_display`,
  `profile_for(version) -> Profile { cpu, experimental, needs_avx2 }`, `root_patch_version`,
  `wide_resolution`, `is_pci_addr`, `clean(path)`, `load_config`/`save_config` (standalone JSON);
  constants `DISPLAY_*`, `GRAPHICS_*`, `RESOLUTIONS`, `DEFAULT_RESOLUTION`, `MIN_RAM_MIB`.
- `network`: `Nic`, `Config::adapters()` (`None` = legacy NAT adapter with `LEGACY_MAC`),
  `.nic_mut(i)`, `.ssh_forwarded()`; `new_networks`, `random_mac`, `nic_supported`,
  `default_nic_model`, `validate_nics`, `net_args(c, helper) -> (first, rest)`, `bridge_helper`,
  `host_bridges`/`bridges_in`, `check_bridges`, `bridge_allowed`, `check_ssh_port`,
  `disk_virtual_size`; `NIC_MODELS`, `MAX_NICS`, `NET_NAT`, `NET_BRIDGED`.
- `settings`: `AppDir` (`user()` = `dirs::config_dir()/macos-kvm-gui`, `at(dir)`, `db_path`,
  `settings_file`, `env_file`, `load_settings`, `reset_settings`, `load_extra_kexts`,
  `setting(key)` env > settings.yaml > .env, `file_setting`), `FileSettings`, `ExtraKext`,
  `ExtraKexts`, `DEFAULT_SETTINGS_YAML`, keys `MKG_THEME`, `MKG_ASSETS_DIR`, `MKG_REIMS_QEMU`.
- `registry`: `Registry::open(&AppDir)` (schema 1→5 migrations, legacy file import), `list`,
  `create`, `save`, `delete(e, purge)`, `selected`, `select`, `preference`/`set_preference`
  (`PREF_*` keys), `workspace_root`, `set_workspace_root`, `vm_folder`, `move_vm`,
  `import_settings`, `import_bundle`; `VmEntry { id, path, config }`, `.display_name()`;
  `find_vm`, `slug`, `export_vm`, `SCHEMA_VERSION`, `SCHEMA_V1`, `DEFAULT_WORKSPACE`.
- `workspace`: `Workspace { dir }` (`path`, `lock() -> WorkspaceLock` (Go-compatible flock),
  `exists`, `has_media`, `has_disk`, `media_version`, `create_disk`, `remove_media`), `lock_path`,
  `real_path`, `allocated`, `free_bytes`, `require_empty`, `write_file_atomic`; layout constants
  `BOOT_DIR`, `EFI_DIR`, `OVMF_CODE`, `VARS_FILE`, `IDENTITY_FILE`, `ROOT_PATCH_MARKER`,
  `SNAPSHOT_FILE`, `DISK_FILE`, `MEDIA_FILE`, `MEDIA_MARKER`, `STAGING_DIR`, `WORKSPACE_FILES`,
  `MIN_DISK_GIB`, `MIN_FREE_GIB`, `GIB`.
- `bundle`: `export_bundle(e, dst, cancel, progress)`, `Registry::import_bundle`,
  `check_standalone_disk`, `BUNDLE_MANIFEST`, `BUNDLE_FILES`.
- `keyboard`: `recovery_keyboard`, `mac_keyboard_id`, `host_keyboard`, `guest_typing_ok`.

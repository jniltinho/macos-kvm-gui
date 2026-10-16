# reims-gui

Manage macOS virtual machines on QEMU/KVM with Reims vGPU: the GUI, the CLI and the MCP server
in one binary. Project: https://github.com/jniltinho/macos-kvm-gui

## Run

    ./reims-gui              # GUI
    ./reims-gui --version

Needs `qemu-system-x86` and `qemu-utils`, an X11 or Wayland session and Vulkan or OpenGL
(`mesa-vulkan-drivers` for software rendering). The `.deb` package pulls these in.

## License

The reims-gui source is GPL-2.0-or-later OR LGPL-3.0-or-later (LICENSE). This binary links
crates licensed Apache-2.0 only, which GPL-2.0 does not allow, so **the binary is distributed
under GPL-3.0-or-later** (LICENSES/GPL-3.0-or-later.txt).

- THIRD_PARTY_NOTICES.md: code adapted from other projects and their licenses.
- THIRD_PARTY_LICENSES: the license text of every Rust crate compiled into the binary.
- The corresponding source (vendored crates and the build Dockerfile included) is
  `reims-gui-<version>-source.tar.gz` on the same GitHub Release.

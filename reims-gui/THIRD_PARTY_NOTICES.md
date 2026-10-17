# Third-party notices

reims-gui is licensed GPL-2.0-or-later OR LGPL-3.0-or-later (see LICENSE). It contains code
adapted from these projects, under their licenses:

| Project | License | Used for |
| --- | --- | --- |
| [steelbrain/reims-vgpu](https://github.com/steelbrain/reims-vgpu) (reims-ctl) | GPL-2.0-or-later OR LGPL-3.0-or-later | VM model, QEMU command line, QMP, runtime, host and GPU checks, key mapping |
| [jniltinho/rustXterm](https://github.com/jniltinho/rustXterm) | MIT | app shell, theme tokens, dialogs, desktop (VNC) view, CLI/MCP structure, Docker build |
| [storytold/photocraft](https://github.com/storytold/photocraft) | MIT OR Apache-2.0 | theme tokens, widgets, icons loader (no PhotoCraft or ArtCraft names or marks) |
| [Lucide](https://lucide.dev) icons, some derived from [Feather](https://feathericons.com) | ISC (Lucide), MIT (Feather) | icons |
| [Noto Sans](https://github.com/notofonts/latin-greek-cyrillic) | OFL-1.1 | UI font |
| [JetBrains Mono](https://github.com/JetBrains/JetBrainsMono) | OFL-1.1 | monospace font |
| macos-kvm-gui (Go) | BSD-3-Clause | behavior reference and boot assets (OpenCore, Lilu, VirtualSMC, WhateverGreen, OVMF keep their own licenses) |

Each copied file keeps a header naming its origin and license.

The license texts these need travel with this file, in `third-party/` next to it in the packages
(in the source tree, where each comes from):

| File | Covers | Source tree |
| --- | --- | --- |
| `third-party/BOOT_ASSETS_NOTICES.md` | the embedded boot assets (OpenCore, Lilu, VirtualSMC, WhateverGreen, OVMF) and the recovery protocol, with their BSD license texts | `assets/BOOT_ASSETS_NOTICES.md` |
| `third-party/OVMF-copyright` | OVMF, the full Debian copyright file | `assets/files/ovmf/copyright` |
| `third-party/LICENSE-lucide.txt` | Lucide (ISC) and Feather (MIT) | `crates/rg-ui/assets/lucide/` |
| `third-party/OFL-NotoSans.txt` | Noto Sans | `crates/rg-ui/assets/fonts/` |
| `third-party/OFL-JetBrainsMono.txt` | JetBrains Mono | `crates/rg-ui/assets/fonts/` |

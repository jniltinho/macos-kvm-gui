# Third-party notices

The app embeds the files below, built by `scripts/fetch-assets.sh` from the sources pinned in `assets.env`. They stay under their own licenses, reproduced here.

| Component | Source | License |
|---|---|---|
| OpenCore (`BOOTx64.efi`, `OpenCore.efi`, `OpenRuntime.efi`, `OpenHfsPlus.efi`) and `config.plist.tmpl` (derived from `Docs/Sample.plist`) | [acidanthera/OpenCorePkg](https://github.com/acidanthera/OpenCorePkg) | BSD-3-Clause |
| Lilu.kext | [acidanthera/Lilu](https://github.com/acidanthera/Lilu) | BSD-3-Clause |
| VirtualSMC.kext | [acidanthera/VirtualSMC](https://github.com/acidanthera/VirtualSMC) | BSD-3-Clause |
| WhateverGreen.kext | [acidanthera/WhateverGreen](https://github.com/acidanthera/WhateverGreen) | BSD-3-Clause |
| OVMF (`OVMF_CODE_4M.fd`, `OVMF_VARS_4M.fd`) | Debian [`ovmf`](https://packages.debian.org/source/edk2) package, from [TianoCore EDK II](https://github.com/tianocore/edk2) | BSD-2-Clause-Patent and others; full text in `internal/assets/files/ovmf/copyright` |
| Recovery download protocol (`internal/core/recovery.go`, ported to Go) | acidanthera's [macrecovery](https://github.com/acidanthera/OpenCorePkg/tree/master/Utilities/macrecovery), Copyright (c) 2019 vit9696 | BSD-3-Clause |

Not embedded, downloaded at run time only when a VM's root-patch mode is turned on: AMFIPass, RSRHelper, RestrictEvents and KDKlessWorkaround, from [OpenCore Legacy Patcher](https://github.com/dortania/OpenCore-Legacy-Patcher)'s repository at the 2.5.1 commit (`payloads/Kexts`), checked against pinned SHA-256 sums. RestrictEvents is acidanthera's (BSD-3-Clause); OCLP's repository is under its own license (BSD-style, Copyright (c) 2020-2025 Dhinak G, Mykola Grymalyuk and individual contributors). This product includes software developed by Dortania and OpenCore Legacy Patcher contributors.

Not embedded, downloaded at run time only before a Tahoe VM on Reims vGPU starts: ReimsVgpu.kext (`Contents/Info.plist` and `Contents/MacOS/ReimsVgpu`), from [steelbrain/reims-util](https://github.com/steelbrain/reims-util/tree/b302389a957c56fad8e5b3b3b16107932ea9e5e9/kext) at commit `b302389a957c56fad8e5b3b3b16107932ea9e5e9`, checked against pinned SHA-256 sums and cached in `<Workspace>/.kexts/reims-util-b302389`. It is under GPL-2.0-or-later OR LGPL-3.0-or-later and contains code from Lilu under BSD-3-Clause; its source is in that repository at that commit.

The apple-smc key string passed to QEMU (`isa-applesmc,osk=…`) and the macOS recovery images downloaded from Apple's servers are not part of this project.

## Reims vGPU

Not embedded and not part of the app's packages. `macos-kvm-gui reims install` downloads, from this repository's separate `reims-<commit>` releases, a build of [steelbrain/reims-vgpu](https://github.com/steelbrain/reims-vgpu)'s QEMU for the host's Ubuntu release (`reims-qemu-<commit>-ubuntu<release>-x86_64.tar.gz`, made by `.github/workflows/reims-release.yml` from `internal/core/reims/Dockerfile`), checked against the SHA-256 in `internal/core/reims/manifest.json`; `macos-kvm-gui reims build` builds the same thing from source on the user's machine instead. Each package holds:

| Component | Source | License |
|---|---|---|
| Reims vGPU library, linked statically into QEMU, and its option ROM `ReimsVgpu.rom` | [steelbrain/reims-vgpu](https://github.com/steelbrain/reims-vgpu) | GPL-2.0-or-later OR LGPL-3.0-or-later |
| QEMU with the `apple-gfx-pci` device, and the firmware files QEMU installs (SeaBIOS, iPXE, EDK II and others, each under its own license) | [steelbrain/qemu-reims-vgpu](https://github.com/steelbrain/qemu-reims-vgpu) (upstream's `vendor/qemu`) | GPL-2.0 (`COPYING`), with parts under compatible licenses |
| SDL3, linked statically into the library | [libsdl-org/SDL](https://github.com/libsdl-org/SDL) through the `sdl3-src` crate | zlib |
| Rust crates linked into the library | crates.io and the git sources upstream's `Cargo.lock` names | each crate's own license (MIT, Apache-2.0 and others), included in the source archive |

The same release holds the corresponding source, `reims-vgpu-src-<commit>.tar.gz`: upstream at that commit, its `vendor/qemu` with the meson subprojects QEMU's build fetches, every Rust crate (`cargo vendor`, SDL3's source among them), the license texts, the Dockerfile the packages were built with, and a `SOURCES.txt` listing the commit of each QEMU firmware source (`roms/`). The source is offered under the same terms for as long as the packages are published.

## OpenCorePkg license

```
Copyright (c) 2016-2017, The HermitCrabs Lab
Copyright (c) 2016-2020, Download-Fritz
Copyright (c) 2017-2020, savvas
Copyright (c) 2016-2020, vit9696

All rights reserved.

Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## Lilu license

```
Copyright (c) 2016-2018, vit9696
All rights reserved.

Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## VirtualSMC license

```
Copyright (c) 2017, vit9696

All rights reserved.

Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## WhateverGreen license

```
Copyright (c) 2018, vit9696

All rights reserved.

Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```


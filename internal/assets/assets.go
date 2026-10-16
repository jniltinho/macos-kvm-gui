// Package assets embeds the boot files built by scripts/fetch-assets.sh from
// BSD-licensed upstreams (OpenCore and kexts by acidanthera, OVMF from Debian).
// See THIRD_PARTY_NOTICES.md.
package assets

import "embed"

// FS holds files/EFI (OpenCore tree), files/ovmf (firmware) and config.plist.tmpl.
//
//go:embed all:files config.plist.tmpl
var FS embed.FS

// Logo is the app icon (original artwork, BSD-3-Clause like the app).
//
//go:embed logo/logo.svg
var Logo []byte

// VersionIcons has one original icon per macOS version, named <short name>.svg.
//
//go:embed logo/versions/*.svg
var VersionIcons embed.FS

// Icons are the app's own toolbar, state and section icons (original artwork, BSD-3-Clause).
//
//go:embed icons/*.svg
var Icons embed.FS

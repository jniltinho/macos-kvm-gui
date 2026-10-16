// Package assets embeds the boot files built by scripts/fetch-assets.sh from
// BSD-licensed upstreams (OpenCore and kexts by acidanthera, OVMF from Debian).
// See THIRD_PARTY_NOTICES.md.
package assets

import "embed"

// FS holds files/EFI (OpenCore tree), files/ovmf (firmware) and config.plist.tmpl.
//
//go:embed all:files config.plist.tmpl
var FS embed.FS

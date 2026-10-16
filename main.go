// macos-kvm-gui installs and runs macOS on Linux KVM: a desktop app (default) and a CLI.
package main

import "github.com/jniltinho/macos-kvm-gui/cmd"

// version is set at build time: go build -ldflags "-X main.version=v0.2.0".
var version = "dev"

func main() { cmd.Execute(version) }

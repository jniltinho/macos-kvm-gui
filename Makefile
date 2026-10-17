BIN     := dist/macos-kvm-gui
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
LDFLAGS := -s -w -X main.version=$(VERSION)
# reims-gui (Rust) builds in Docker (Ubuntu 24.04 image), with the same target dir as its build.sh.
RG_VERSION := $(shell sed -n 's/^version = "\(.*\)"$$/\1/p' reims-gui/Cargo.toml | head -n 1)
RG_TARGET  := target/release-24.04

# Binaries go to dist/, which is git-ignored: nothing built is ever committed.
# `make build` leaves both apps there: dist/macos-kvm-gui (Go) and dist/reims-gui (Rust).
build: build-go build-rust

# The Go app embeds Reims vGPU's window through reims-gui's rg-x11 (C ABI, crates/rg-x11-ffi),
# built in Docker (Ubuntu 24.04 image) into the one path internal/embed links. Plain `go build`
# without -tags reims_embed needs none of it and keeps the window standalone.
EMBED_LIB := reims-gui/target/rg-x11-ffi/release/librg_x11_ffi.a
GO_TAGS   := -tags reims_embed

# The archive is pruned to the rgx_* functions and what they use (ld -r --gc-sections): Rust's
# std otherwise brings weak pidfd_* references that the Ubuntu 24.04 linker binds to GLIBC_2.39,
# raising the Go binary's floor above the 2.38 its packages declare (Debian 13, Fedora 39).
embed-lib:
	cd reims-gui && CARGO_TARGET_DIR=target/rg-x11-ffi scripts/cargo-docker.sh 24.04 sh -c '\
		set -e; cargo build --release --locked -p rg-x11-ffi; cd target/rg-x11-ffi/release; \
		roots=$$(nm -g --defined-only librg_x11_ffi.a 2>/dev/null | awk "/ T rgx_/{print \"-u \" \$$3}" | sort -u); \
		ld -r --gc-sections $$roots --whole-archive librg_x11_ffi.a -o rg_x11_ffi.o; \
		rm -f librg_x11_ffi.a; ar rcs librg_x11_ffi.a rg_x11_ffi.o; rm rg_x11_ffi.o'

build-go: embed-lib
	mkdir -p dist
	go build $(GO_TAGS) -o $(BIN) .

build-rust:
	mkdir -p dist
	cd reims-gui && CARGO_TARGET_DIR=$(RG_TARGET) scripts/cargo-docker.sh 24.04 cargo build --release --locked -p reims-gui
	cp reims-gui/$(RG_TARGET)/release/reims-gui dist/reims-gui

# Stripped, reproducible-path binary with the version baked in (used by releases).
build-prod: embed-lib
	mkdir -p dist
	go build $(GO_TAGS) -trimpath -ldflags "$(LDFLAGS)" -o $(BIN) .

# OpenCore's validator for the config.plist tests; downloaded once, never committed.
internal/core/testdata/ocvalidate:
	scripts/fetch-ocvalidate.sh

test: internal/core/testdata/ocvalidate
	test -z "$$(gofmt -l .)" || { gofmt -l .; echo "run gofmt -w ."; exit 1; }
	go vet ./...
	go test -race ./...

# internal/embed's cgo wrapper against the real library, under Xvfb.
test-embed: embed-lib
	cd reims-gui && scripts/cargo-docker.sh 24.04 cargo build --locked -p rg-x11 --example sdl_window
	go test $(GO_TAGS) -count=1 -v -run . ./internal/embed

# Real QEMU/KVM boot of the embedded OpenCore in a temporary workspace.
test-integration:
	go test -race -tags integration -run Real ./internal/core

# dist/: both apps' packages and SHA-256 files.
package: package-go package-rust

# The Go app's .tar.gz, .deb and .rpm for VERSION.
package-go: build-prod
	scripts/package.sh $(VERSION) $(BIN)

# reims-gui's .deb and .tar.gz (one for Ubuntu 24.04 and 26.04), built in Docker.
package-rust:
	mkdir -p dist
	reims-gui/scripts/build.sh
	cp reims-gui/dist/reims-gui_$(RG_VERSION)_amd64.deb* reims-gui/dist/reims-gui-$(RG_VERSION)-x86_64.tar.gz* dist/

run: build-go
	./$(BIN)

clean:
	rm -rf dist

.PHONY: build build-go build-rust build-prod embed-lib test-embed test test-integration package package-go package-rust run clean

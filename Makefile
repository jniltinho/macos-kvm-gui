BIN     := dist/macos-kvm-gui
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
LDFLAGS := -s -w -X main.version=$(VERSION)

# Binaries go to dist/, which is git-ignored: nothing built is ever committed.
build:
	mkdir -p dist
	go build -o $(BIN) .

# Stripped, reproducible-path binary with the version baked in (used by releases).
build-prod:
	mkdir -p dist
	go build -trimpath -ldflags "$(LDFLAGS)" -o $(BIN) .

# OpenCore's validator for the config.plist tests; downloaded once, never committed.
internal/core/testdata/ocvalidate:
	scripts/fetch-ocvalidate.sh

test: internal/core/testdata/ocvalidate
	test -z "$$(gofmt -l .)" || { gofmt -l .; echo "run gofmt -w ."; exit 1; }
	go vet ./...
	go test -race ./...

# Real QEMU/KVM boot of the embedded OpenCore in a temporary workspace.
test-integration:
	go test -race -tags integration -run Real ./internal/core

# dist/: .tar.gz, .deb and SHA-256 files for VERSION.
package: build-prod
	scripts/package.sh $(VERSION) $(BIN)

run: build
	./$(BIN)

clean:
	rm -rf dist

.PHONY: build build-prod test test-integration package run clean

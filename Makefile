BIN     := macos-kvm-gui
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
LDFLAGS := -s -w -X main.version=$(VERSION)

build:
	go build -o $(BIN) .

# Stripped, reproducible-path binary with the version baked in (used by releases).
build-prod:
	go build -trimpath -ldflags "$(LDFLAGS)" -o $(BIN) .

test:
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
	rm -rf $(BIN) dist

.PHONY: build build-prod test test-integration package run clean

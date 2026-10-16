BIN := macos-kvm-gui

build:
	go build -o $(BIN) .

test:
	go vet ./...
	go test -race ./...

# QEMU/KVM reais; usa ~/OSX-KVM (ou $$OSX_KVM) só para leitura
test-integration:
	go test -race -tags integration -run Real ./internal/core

run: build
	./$(BIN)

.PHONY: build test test-integration run

BIN_DIR := bin
POLOD    := $(BIN_DIR)/polod
POLO     := $(BIN_DIR)/polo

.PHONY: all build test lint fmt clean docker-build docker-up docker-down

all: build

build:
	@mkdir -p $(BIN_DIR)
	cargo build --release
	@cp target/release/polod $(POLOD)
	@cp target/release/polo  $(POLO)

build-dev:
	@mkdir -p $(BIN_DIR)
	cargo build
	@cp target/debug/polod $(POLOD)
	@cp target/debug/polo  $(POLO)

test:
	cargo test --workspace

test-verbose:
	cargo test --workspace -- --nocapture

lint:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

clean:
	cargo clean
	rm -rf $(BIN_DIR)

docker-build:
	docker build -t polo:latest .

docker-up:
	docker compose up -d

docker-down:
	docker compose down

# Run a local dev server against a temp db
run-dev:
	@mkdir -p $(BIN_DIR)
	cargo build 2>/dev/null
	@cp target/debug/polod $(POLOD)
	RUST_LOG=polo=debug $(POLOD) --db ./polo-dev.db --addr 127.0.0.1:5432

.PHONY: help
help:
	@echo "Targets:"
	@echo "  build        release build, outputs to bin/"
	@echo "  build-dev    debug build"
	@echo "  test         run all tests"
	@echo "  lint         clippy"
	@echo "  fmt          format"
	@echo "  clean        remove artifacts"
	@echo "  docker-build build docker image"
	@echo "  docker-up    start via compose"
	@echo "  docker-down  stop compose stack"
	@echo "  run-dev      local dev server on :5432"

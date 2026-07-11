# Model Browser — Development Tasks

default_dir := "/mnt/d/inplace"
port := "8080"

# Run in debug mode with default library path
dev:
    cargo run -- --dir {{default_dir}} --port {{port}}

# Run in release mode
run:
    cargo run --release -- --dir {{default_dir}} --port {{port}}

# Build debug
build:
    cargo build

# Build release
release:
    cargo build --release

# Check + clippy
check:
    cargo check
    cargo clippy -- -D warnings

# Run tests
test:
    cargo test

# Format code
fmt:
    cargo fmt

# Format check (CI)
fmt-check:
    cargo fmt -- --check

# Clean build artifacts
clean:
    cargo clean

# Download three.js vendor files (also done automatically by build.rs)
vendor-js:
    mkdir -p frontend/vendor
    curl -sL "https://cdn.jsdelivr.net/npm/three@0.166.1/build/three.module.min.js" -o frontend/vendor/three.module.js
    curl -sL "https://cdn.jsdelivr.net/npm/three@0.166.1/examples/jsm/controls/OrbitControls.js" -o frontend/vendor/OrbitControls.js

# Cross-compile for Windows (requires mingw-w64)
windows:
    rustup target add x86_64-pc-windows-gnu
    cargo build --release --target x86_64-pc-windows-gnu

# Full CI check
ci: fmt-check check test

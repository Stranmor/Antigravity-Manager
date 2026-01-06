# Antigravity Manager - Task Runner
# Usage: just <recipe>
# List all recipes: just --list

set shell := ["bash", "-uc"]

# Default recipe - show available commands
default:
    @just --list

# ============================================================================
# BUILD RECIPES
# ============================================================================

# Build Tauri desktop app (release)
build:
    cd src-tauri && cargo build --release

# Build headless server binary
build-server:
    cd src-tauri && cargo build --release --no-default-features --features headless --bin antigravity-server

# Build Slint native UI
build-slint:
    cd src-slint && cargo build --release

# Build with release-fast profile (faster builds, still optimized)
build-fast:
    cd src-tauri && cargo build --profile release-fast

# Build all targets
build-all: build build-server build-slint

# ============================================================================
# RUN RECIPES
# ============================================================================

# Run Tauri development mode
run:
    cd src-tauri && cargo tauri dev

# Run headless server
run-server:
    cd src-tauri && cargo run --release --no-default-features --features headless --bin antigravity-server -- serve

# Run Slint native UI
run-slint:
    cd src-slint && cargo run --release

# ============================================================================
# TEST & CHECK RECIPES
# ============================================================================

# Run all tests
test:
    cd src-tauri && cargo test --all-features

# Run cargo check on all targets
check:
    cd src-tauri && cargo check --all-features
    cd src-slint && cargo check

# Run clippy linter
lint:
    cd src-tauri && cargo clippy --all-features -- -D warnings
    cd src-slint && cargo clippy -- -D warnings

# Format all Rust code
fmt:
    cd src-tauri && cargo fmt
    cd src-slint && cargo fmt

# Check formatting without modifying
fmt-check:
    cd src-tauri && cargo fmt --check
    cd src-slint && cargo fmt --check

# ============================================================================
# DEPLOY RECIPES
# ============================================================================

# VPS target host
vps := "vps-production"

# Build container and deploy to VPS
deploy-vps: build-server
    #!/usr/bin/env bash
    set -euo pipefail
    echo "==> Building container image..."
    podman build -t localhost/antigravity-server:latest -f Containerfile .
    echo "==> Saving and transferring image to VPS..."
    podman save localhost/antigravity-server:latest | ssh {{vps}} "podman load"
    echo "==> Restarting service on VPS..."
    ssh {{vps}} "sudo systemctl restart antigravity-server"
    echo "==> Checking service status..."
    ssh {{vps}} "sudo systemctl status antigravity-server --no-pager"
    echo "==> Deploy complete!"

# Sync binary to VPS without container rebuild
sync-vps: build-server
    #!/usr/bin/env bash
    set -euo pipefail
    echo "==> Copying binary to VPS..."
    scp src-tauri/target/release/antigravity-server {{vps}}:/tmp/
    ssh {{vps}} "sudo mv /tmp/antigravity-server /usr/local/bin/ && sudo systemctl restart antigravity-server"
    echo "==> Sync complete!"

# Check VPS service status
vps-status:
    ssh {{vps}} "curl -s http://localhost:9101/api/health | jq"

# View VPS logs (follow mode)
vps-logs:
    ssh {{vps}} "sudo journalctl -u antigravity-server -f"

# ============================================================================
# UTILITY RECIPES
# ============================================================================

# Clean all build artifacts
clean:
    cd src-tauri && cargo clean
    cd src-slint && cargo clean

# Update all dependencies
update:
    cd src-tauri && cargo update
    cd src-slint && cargo update

# Show binary sizes
sizes:
    @echo "==> Binary sizes:"
    @ls -lh src-tauri/target/release/antigravity_tools 2>/dev/null || echo "  Desktop: not built"
    @ls -lh src-tauri/target/release/antigravity-server 2>/dev/null || echo "  Server: not built"
    @ls -lh src-slint/target/release/antigravity-desktop 2>/dev/null || echo "  Slint: not built"

# Import accounts from desktop app to server
import-accounts:
    cd src-tauri && cargo run --release --no-default-features --features headless --bin antigravity-server -- import

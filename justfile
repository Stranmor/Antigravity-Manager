# Antigravity Manager - Build Commands
# Usage: just <command>

set shell := ["zsh", "-cu"]
set dotenv-load := true

# Default: show help
default:
    @just --list

# === Development ===

# Start development server with hot reload
dev:
    cargo tauri dev

# Run clippy on all packages
lint:
    cargo clippy --workspace -- -D warnings

# Format all code
fmt:
    cargo fmt --all

# Check all packages compile
check:
    cargo check --workspace

# === Frontend (Leptos) ===

# Build Leptos frontend (dev)
frontend-dev:
    cd src-leptos && trunk build

# Build Leptos frontend (release)
frontend-release:
    cd src-leptos && trunk build --release

# Serve Leptos frontend for development
frontend-serve:
    cd src-leptos && trunk serve --port 1420

# === Production Build ===

# Build release binary for current platform
build:
    cargo tauri build

# Build release with verbose output
build-verbose:
    cargo tauri build --verbose

# Build for specific target (e.g., just build-target aarch64-apple-darwin)
build-target target:
    cargo tauri build --target {{target}}

# === Linux Specific ===

# Build AppImage (Linux)
build-appimage:
    cargo tauri build --bundles appimage

# Build .deb package (Linux)
build-deb:
    cargo tauri build --bundles deb

# === macOS Specific ===

# Build DMG (macOS)
build-dmg:
    cargo tauri build --bundles dmg

# Build Universal macOS binary
build-universal:
    cargo tauri build --target universal-apple-darwin

# === Windows Specific ===

# Build MSI installer (Windows)
build-msi:
    cargo tauri build --bundles msi

# Build NSIS installer (Windows)
build-nsis:
    cargo tauri build --bundles nsis

# === Testing ===

# Run all tests
test:
    cargo test --workspace

# Run tests with output
test-verbose:
    cargo test --workspace -- --nocapture

# === Utilities ===

# Clean all build artifacts
clean:
    cargo clean
    rm -rf src-leptos/dist

# Update all dependencies
update:
    cargo update

# Show outdated dependencies
outdated:
    cargo outdated -R

# Generate icons from 1024x1024 source
icons source="icons/app-icon.png":
    cargo tauri icon {{source}}

# === Deployment ===

# Build and copy to remote server
deploy-vps: build
    rsync -avz --progress target/release/bundle/ vps-production:/opt/antigravity/

# === Quick Commands ===

# Full rebuild: clean + build
rebuild: clean build

# Pre-commit check: fmt + lint + test
pre-commit: fmt lint test

# === Installation ===

# Install the latest built deb package
install:
    sudo dpkg -i target/release/bundle/deb/*.deb

# Build and install in one command
reinstall: build-deb install

# Uninstall
uninstall:
    sudo dpkg -r antigravity-tools

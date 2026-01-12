#!/usr/bin/env bash
# Install git hooks for Antigravity Manager
# Run this script once after cloning the repository

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
HOOKS_DIR="$PROJECT_ROOT/.githooks"
GIT_HOOKS_DIR="$PROJECT_ROOT/.git/hooks"

echo "Installing git hooks..."

# Configure git to use .githooks directory
git config core.hooksPath .githooks

# Ensure hooks are executable
chmod +x "$HOOKS_DIR"/*

echo "✓ Git hooks installed successfully!"
echo "  Hooks directory: .githooks/"
echo "  Active hooks: pre-commit"

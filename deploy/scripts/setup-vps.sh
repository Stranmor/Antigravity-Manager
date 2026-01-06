#!/bin/bash
# Antigravity Server - VPS Setup Script
# Installs and configures Antigravity Server using Podman Quadlets
#
# Supports: Debian 12+, Ubuntu 22.04+, RHEL 9+, Fedora 39+, Arch Linux
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/lbjlaq/Antigravity-Manager/main/deploy/scripts/setup-vps.sh | bash
#   # or
#   ./setup-vps.sh [--build] [--warp]
#
# Options:
#   --build     Build image locally instead of pulling
#   --warp      Enable WARP proxy (Cloudflare)
#   --port PORT Override default port (default: 8045)

set -euo pipefail

# Configuration
ANTIGRAVITY_VERSION="${ANTIGRAVITY_VERSION:-latest}"
ANTIGRAVITY_PORT="${ANTIGRAVITY_PORT:-8045}"
ANTIGRAVITY_ADMIN_PORT="${ANTIGRAVITY_ADMIN_PORT:-9101}"
ANTIGRAVITY_DATA_DIR="${ANTIGRAVITY_DATA_DIR:-/var/lib/antigravity}"
QUADLET_DIR="/etc/containers/systemd"
BUILD_LOCAL=false
ENABLE_WARP=false

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1" >&2; }
log_step() { echo -e "${BLUE}[STEP]${NC} $1"; }

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --build)
            BUILD_LOCAL=true
            shift
            ;;
        --warp)
            ENABLE_WARP=true
            shift
            ;;
        --port)
            ANTIGRAVITY_PORT="$2"
            shift 2
            ;;
        --admin-port)
            ANTIGRAVITY_ADMIN_PORT="$2"
            shift 2
            ;;
        --data-dir)
            ANTIGRAVITY_DATA_DIR="$2"
            shift 2
            ;;
        *)
            log_error "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Check root
check_root() {
    if [[ $EUID -ne 0 ]]; then
        log_error "This script must be run as root (use sudo)"
        exit 1
    fi
}

# Detect OS
detect_os() {
    if [[ -f /etc/os-release ]]; then
        . /etc/os-release
        OS="${ID}"
        OS_VERSION="${VERSION_ID:-}"
        OS_FAMILY="${ID_LIKE:-$ID}"
    else
        log_error "Cannot detect OS"
        exit 1
    fi
    log_info "Detected OS: ${OS} ${OS_VERSION}"
}

# Install Podman
install_podman() {
    log_step "Installing Podman..."

    case "${OS}" in
        debian|ubuntu)
            apt-get update -qq
            apt-get install -y -qq podman curl jq
            ;;
        fedora)
            dnf install -y -q podman curl jq
            ;;
        rhel|centos|rocky|almalinux)
            dnf install -y -q podman curl jq
            ;;
        arch|manjaro)
            pacman -Syu --noconfirm podman curl jq
            ;;
        alpine)
            apk add --no-cache podman curl jq
            ;;
        *)
            log_error "Unsupported OS: ${OS}"
            log_info "Please install Podman manually: https://podman.io/getting-started/installation"
            exit 1
            ;;
    esac

    # Verify installation
    if ! command -v podman &> /dev/null; then
        log_error "Podman installation failed"
        exit 1
    fi

    log_info "Podman version: $(podman --version)"
}

# Create data directories
create_directories() {
    log_step "Creating data directories..."

    mkdir -p "${ANTIGRAVITY_DATA_DIR}/accounts"
    mkdir -p "${ANTIGRAVITY_DATA_DIR}/logs"
    mkdir -p "${QUADLET_DIR}"
    mkdir -p /etc/antigravity

    # Set permissions
    chmod 700 "${ANTIGRAVITY_DATA_DIR}"
    chmod 700 "${ANTIGRAVITY_DATA_DIR}/accounts"

    log_info "Data directory: ${ANTIGRAVITY_DATA_DIR}"
}

# Generate API key
generate_api_key() {
    if [[ -z "${ANTIGRAVITY_API_KEY:-}" ]]; then
        ANTIGRAVITY_API_KEY="sk-$(openssl rand -hex 32)"
        log_info "Generated API key: ${ANTIGRAVITY_API_KEY}"
    fi
}

# Create initial config
create_config() {
    log_step "Creating configuration..."

    local CONFIG_FILE="${ANTIGRAVITY_DATA_DIR}/config.json"

    if [[ -f "${CONFIG_FILE}" ]]; then
        log_warn "Config already exists, skipping..."
        return
    fi

    cat > "${CONFIG_FILE}" << EOF
{
  "proxy": {
    "enabled": true,
    "allow_lan_access": true,
    "auth_mode": "all_except_health",
    "port": ${ANTIGRAVITY_PORT},
    "api_key": "${ANTIGRAVITY_API_KEY}",
    "auto_start": true,
    "request_timeout": 300,
    "enable_logging": true,
    "anthropic_mapping": {},
    "openai_mapping": {},
    "custom_mapping": {},
    "upstream_proxy": {
      "enabled": false,
      "url": ""
    },
    "zai": {
      "enabled": false,
      "base_url": "https://api.z.ai/api/anthropic",
      "api_key": "",
      "dispatch_mode": "off"
    }
  }
}
EOF

    chmod 600 "${CONFIG_FILE}"
    log_info "Config created: ${CONFIG_FILE}"
}

# Create environment file
create_env_file() {
    log_step "Creating environment file..."

    cat > /etc/antigravity/env << EOF
# Antigravity Server Environment
# Generated on $(date -Iseconds)

ANTIGRAVITY_DATA_DIR=${ANTIGRAVITY_DATA_DIR}
ANTIGRAVITY_PROXY_PORT=${ANTIGRAVITY_PORT}
ANTIGRAVITY_ADMIN_PORT=${ANTIGRAVITY_ADMIN_PORT}
ANTIGRAVITY_ALLOW_LAN=true
ANTIGRAVITY_ENABLE_LOGGING=true
ANTIGRAVITY_API_KEY=${ANTIGRAVITY_API_KEY}
RUST_LOG=info,antigravity_tools_lib=debug
TZ=UTC
EOF

    if [[ "${ENABLE_WARP}" == "true" ]]; then
        echo "WARP_ENABLED=true" >> /etc/antigravity/env
    fi

    chmod 600 /etc/antigravity/env
    log_info "Environment file: /etc/antigravity/env"
}

# Install Quadlet files
install_quadlets() {
    log_step "Installing Quadlet systemd units..."

    # Volume unit
    cat > "${QUADLET_DIR}/antigravity-data.volume" << 'EOF'
[Volume]
VolumeName=antigravity-data
Driver=local
Label=app=antigravity
Label=component=data
EOF

    # Container unit
    cat > "${QUADLET_DIR}/antigravity-server.container" << EOF
[Unit]
Description=Antigravity Proxy Server
Documentation=https://github.com/lbjlaq/Antigravity-Manager
After=network-online.target
Wants=network-online.target

[Container]
Image=localhost/antigravity-server:latest
ContainerName=antigravity-server
AutoUpdate=registry

PublishPort=${ANTIGRAVITY_PORT}:8045
PublishPort=${ANTIGRAVITY_ADMIN_PORT}:9101

EnvironmentFile=/etc/antigravity/env
Volume=antigravity-data.volume:/var/lib/antigravity:Z

PodmanArgs=--memory=512m
PodmanArgs=--cpus=2.0

NoNewPrivileges=true
DropCapability=ALL
AddCapability=NET_BIND_SERVICE

HealthCmd=curl -sf http://localhost:8045/healthz || exit 1
HealthInterval=30s
HealthTimeout=5s
HealthRetries=3
HealthStartPeriod=10s

LogDriver=journald
User=1000
Group=1000

[Service]
Restart=always
RestartSec=10
TimeoutStartSec=300
TimeoutStopSec=30
Type=notify
NotifyAccess=all

[Install]
WantedBy=multi-user.target default.target
EOF

    log_info "Quadlet files installed to ${QUADLET_DIR}"
}

# Build or pull image
setup_image() {
    log_step "Setting up container image..."

    if [[ "${BUILD_LOCAL}" == "true" ]]; then
        log_info "Building image locally..."

        # Clone repository if not present
        local REPO_DIR="/tmp/antigravity-manager"
        if [[ ! -d "${REPO_DIR}" ]]; then
            git clone --depth=1 https://github.com/lbjlaq/Antigravity-Manager.git "${REPO_DIR}"
        fi

        cd "${REPO_DIR}"
        podman build -t localhost/antigravity-server:latest .

        log_info "Image built successfully"
    else
        log_info "Pulling pre-built image..."
        # Note: Replace with actual registry when available
        log_warn "Pre-built images not yet available. Building locally..."
        BUILD_LOCAL=true
        setup_image
    fi
}

# Start service
start_service() {
    log_step "Starting Antigravity Server..."

    # Reload systemd
    systemctl daemon-reload

    # Enable and start
    systemctl enable --now antigravity-server.service

    # Wait for health check
    log_info "Waiting for server to be ready..."
    local max_attempts=30
    local attempt=0

    while [[ $attempt -lt $max_attempts ]]; do
        if curl -sf "http://localhost:${ANTIGRAVITY_PORT}/healthz" > /dev/null 2>&1; then
            log_info "Server is ready!"
            break
        fi
        sleep 1
        ((attempt++))
    done

    if [[ $attempt -ge $max_attempts ]]; then
        log_warn "Server may not be ready yet. Check: systemctl status antigravity-server"
    fi
}

# Print summary
print_summary() {
    echo ""
    echo "=============================================="
    echo -e "${GREEN}Antigravity Server Installation Complete!${NC}"
    echo "=============================================="
    echo ""
    echo "Configuration:"
    echo "  Data Directory: ${ANTIGRAVITY_DATA_DIR}"
    echo "  Proxy Port:     ${ANTIGRAVITY_PORT}"
    echo "  Admin Port:     ${ANTIGRAVITY_ADMIN_PORT}"
    echo "  API Key:        ${ANTIGRAVITY_API_KEY}"
    echo ""
    echo "API Endpoints:"
    echo "  OpenAI API:    http://localhost:${ANTIGRAVITY_PORT}/v1/chat/completions"
    echo "  Claude API:    http://localhost:${ANTIGRAVITY_PORT}/v1/messages"
    echo "  Health Check:  http://localhost:${ANTIGRAVITY_PORT}/healthz"
    echo ""
    echo "Useful Commands:"
    echo "  View logs:     journalctl -u antigravity-server -f"
    echo "  Restart:       systemctl restart antigravity-server"
    echo "  Stop:          systemctl stop antigravity-server"
    echo "  Status:        systemctl status antigravity-server"
    echo ""
    echo "Add Accounts:"
    echo "  Place account JSON files in: ${ANTIGRAVITY_DATA_DIR}/accounts/"
    echo "  Then restart: systemctl restart antigravity-server"
    echo ""
    if [[ "${ENABLE_WARP}" == "true" ]]; then
        echo -e "${YELLOW}WARP enabled - Cloudflare proxy will be used for upstream requests${NC}"
        echo ""
    fi
    echo -e "${YELLOW}IMPORTANT: Save your API key securely!${NC}"
    echo ""
}

# Main
main() {
    echo ""
    echo "=============================================="
    echo "  Antigravity Server VPS Setup"
    echo "=============================================="
    echo ""

    check_root
    detect_os

    # Check if podman is installed
    if ! command -v podman &> /dev/null; then
        install_podman
    else
        log_info "Podman already installed: $(podman --version)"
    fi

    create_directories
    generate_api_key
    create_config
    create_env_file
    install_quadlets
    setup_image
    start_service
    print_summary
}

main "$@"

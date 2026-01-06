#!/bin/sh
# Antigravity Server Entrypoint
# Handles WARP initialization and server startup

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Initialize data directory structure
init_data_dir() {
    log_info "Initializing data directory: ${ANTIGRAVITY_DATA_DIR}"

    mkdir -p "${ANTIGRAVITY_DATA_DIR}/accounts"
    mkdir -p "${ANTIGRAVITY_DATA_DIR}/logs"

    # Create default config if not exists
    if [ ! -f "${ANTIGRAVITY_DATA_DIR}/config.json" ]; then
        log_info "Creating default config.json"
        cat > "${ANTIGRAVITY_DATA_DIR}/config.json" << 'EOF'
{
  "proxy": {
    "enabled": true,
    "allow_lan_access": true,
    "auth_mode": "all_except_health",
    "port": 8045,
    "api_key": "",
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
    fi
}

# Initialize WARP if enabled
init_warp() {
    if [ "${WARP_ENABLED:-false}" = "true" ]; then
        log_info "WARP enabled, checking configuration..."

        WARP_CONFIG="${ANTIGRAVITY_DATA_DIR}/warp.conf"

        if [ ! -f "$WARP_CONFIG" ]; then
            log_info "Generating WARP configuration..."

            # Accept WARP TOS if needed
            if [ ! -f "${ANTIGRAVITY_DATA_DIR}/wgcf-account.toml" ]; then
                cd "${ANTIGRAVITY_DATA_DIR}"
                wgcf register --accept-tos
            fi

            # Generate WireGuard config
            cd "${ANTIGRAVITY_DATA_DIR}"
            wgcf generate

            # Convert to wireproxy format
            cat > "$WARP_CONFIG" << EOF
[Interface]
PrivateKey = $(grep PrivateKey wgcf-profile.conf | cut -d' ' -f3)
DNS = 1.1.1.1
MTU = 1280

[Peer]
PublicKey = $(grep PublicKey wgcf-profile.conf | cut -d' ' -f3)
Endpoint = $(grep Endpoint wgcf-profile.conf | cut -d' ' -f3)
AllowedIPs = 0.0.0.0/0

[Socks5]
BindAddress = 127.0.0.1:1080
EOF
            log_info "WARP configuration generated"
        fi

        # Start wireproxy in background
        log_info "Starting wireproxy SOCKS5 proxy on 127.0.0.1:1080..."
        wireproxy -c "$WARP_CONFIG" &
        WIREPROXY_PID=$!

        # Wait for proxy to be ready
        sleep 2

        if kill -0 $WIREPROXY_PID 2>/dev/null; then
            log_info "WARP proxy started successfully (PID: $WIREPROXY_PID)"

            # Configure upstream proxy to use WARP
            export ANTIGRAVITY_UPSTREAM_PROXY="socks5://127.0.0.1:1080"
        else
            log_error "Failed to start WARP proxy"
        fi
    fi
}

# Graceful shutdown handler
cleanup() {
    log_info "Shutting down..."

    # Kill wireproxy if running
    if [ -n "$WIREPROXY_PID" ] && kill -0 $WIREPROXY_PID 2>/dev/null; then
        kill $WIREPROXY_PID
    fi

    exit 0
}

trap cleanup SIGTERM SIGINT

# Main entrypoint
main() {
    log_info "Antigravity Server starting..."
    log_info "Data directory: ${ANTIGRAVITY_DATA_DIR}"
    log_info "Proxy port: ${ANTIGRAVITY_PROXY_PORT:-8045}"

    # Initialize
    init_data_dir
    init_warp

    # Show account count
    ACCOUNT_COUNT=$(find "${ANTIGRAVITY_DATA_DIR}/accounts" -name "*.json" 2>/dev/null | wc -l)
    log_info "Found ${ACCOUNT_COUNT} account files"

    if [ "$ACCOUNT_COUNT" -eq 0 ]; then
        log_warn "No accounts found! Add account JSON files to ${ANTIGRAVITY_DATA_DIR}/accounts/"
    fi

    # Execute command (default: antigravity-server)
    log_info "Starting server..."
    exec "$@"
}

main "$@"

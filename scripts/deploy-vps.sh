#!/bin/bash
set -e

# Antigravity Manager - VPS Deployment Script (v2026.01.07)
# Automates Build → Ship → Deploy → Verify cycle for VPS production.

# --- Configuration ---
VPS_HOST="vps-production"
IMAGE_NAME="antigravity-server"
ADMIN_PORT=9101
HEALTH_CHECK_ENDPOINT="/api/health/detailed"
TAR_PATH="/tmp/${IMAGE_NAME}.tar.gz"
PROJECT_ROOT="/home/stranmor/Documents/project/_mycelium/Antigravity-Manager"

# --- Flags ---
SKIP_BUILD=false
for arg in "$@"; do
    if [ "$arg" == "--skip-build" ]; then
        SKIP_BUILD=true
    fi
done

# --- Execution ---
echo "🚀 Starting deployment to ${VPS_HOST}..."

# Ensure we are in the project root
cd "${PROJECT_ROOT}"

if [ "$SKIP_BUILD" = false ]; then
    echo "📦 Building podman image (Multi-stage build including headless binary)..."
    podman build -t "${IMAGE_NAME}:latest" -f "${PROJECT_ROOT}/Containerfile" "${PROJECT_ROOT}"

    echo "📤 Saving image to tarball: ${TAR_PATH}..."
    podman save "${IMAGE_NAME}:latest" | gzip > "${TAR_PATH}"
else
    echo "⏭️ Skipping build as requested."
    if [ ! -f "${TAR_PATH}" ]; then
        echo "❌ Error: ${TAR_PATH} not found. Cannot skip build if image tarball doesn't exist."
        exit 1
    fi
fi

echo "🚢 Shipping image to VPS..."
# Pipe directly to ssh to avoid storing large files on VPS disk before load
cat "${TAR_PATH}" | ssh "${VPS_HOST}" "podman load"

echo "🔄 Restarting service on VPS via Systemd Quadlet..."
ssh "${VPS_HOST}" "systemctl restart antigravity-server"

echo "🧪 Verifying health check..."
# Give the container a moment to start
sleep 5

MAX_RETRIES=10
RETRY_COUNT=0
HEALTHY=false

while [ $RETRY_COUNT -lt $MAX_RETRIES ]; do
    echo "Attempt $((RETRY_COUNT+1)) to check health..."
    # Check health via the detailed endpoint on the admin port
    # Returns 200 OK only if all components (DB, TokenManager, etc.) are healthy
    STATUS=$(ssh "${VPS_HOST}" "curl -s -o /dev/null -w '%{http_code}' http://localhost:${ADMIN_PORT}${HEALTH_CHECK_ENDPOINT}")

    if [ "$STATUS" == "200" ]; then
        echo "✅ Health check passed (HTTP 200)!"
        HEALTHY=true
        break
    fi

    echo "⏳ Waiting for service to become healthy (Current status: ${STATUS})..."
    RETRY_COUNT=$((RETRY_COUNT+1))
    if [ $RETRY_COUNT -lt $MAX_RETRIES ]; then
        sleep 10
    fi
done

if [ "$HEALTHY" = false ]; then
    echo "❌ Deployment failed: Service is not healthy after $((MAX_RETRIES * 10)) seconds."
    echo "🔍 Checking logs for clues..."
    ssh "${VPS_HOST}" "journalctl -u antigravity-server --since '5 minutes ago' | tail -n 50"
    exit 1
fi

echo "🎉 Deployment successful and verified!"

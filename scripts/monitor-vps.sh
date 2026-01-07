#!/usr/bin/env bash
# VPS Health Monitor with Auto-Restart
# Runs every 5 minutes via cron to ensure 24/7 availability

set -euo pipefail

# Configuration
HEALTH_URL="https://antigravity.quantumind.ru/healthz"
FAILURES_FILE="/tmp/antigravity-health-failures"
MAX_FAILURES=2  # Restart after 2 consecutive failures (10 minutes)
TIMEOUT=10      # Health check timeout in seconds
VPS_HOST="vps-production"

# Logging with timestamp
log() {
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

# Fetch health status with timeout
fetch_health() {
    curl -s -w "\n%{http_code}" --max-time "$TIMEOUT" "$HEALTH_URL" 2>&1 || echo "000"
}

# Parse response (last line is HTTP code, rest is body)
response=$(fetch_health)
http_code=$(echo "$response" | tail -n1)
body=$(echo "$response" | head -n-1)

# Load failure counter
failures=$(cat "$FAILURES_FILE" 2>/dev/null || echo 0)

# Health check logic
if [ "$http_code" != "200" ]; then
    # HTTP failure
    failures=$((failures + 1))
    log "❌ Health check failed: HTTP $http_code (attempt $failures/$MAX_FAILURES)"
    echo "$failures" > "$FAILURES_FILE"
    
    if [ "$failures" -ge "$MAX_FAILURES" ]; then
        log "🔄 Max failures reached. Restarting antigravity-server on $VPS_HOST..."
        
        # Restart service
        if ssh "$VPS_HOST" "sudo systemctl restart antigravity-server"; then
            log "✅ Service restarted successfully"
            
            # Wait 10 seconds for service to start
            sleep 10
            
            # Verify restart
            restart_response=$(fetch_health)
            restart_code=$(echo "$restart_response" | tail -n1)
            
            if [ "$restart_code" = "200" ]; then
                log "✅ Service verified operational after restart"
                echo "0" > "$FAILURES_FILE"
            else
                log "⚠️  Service restarted but health check still failing (HTTP $restart_code)"
                # Keep failure count to retry on next run
            fi
        else
            log "❌ Failed to restart service via SSH"
            # Keep failure count to retry on next run
        fi
    fi
else
    # Check JSON body for degraded status
    status=$(echo "$body" | jq -r '.status' 2>/dev/null || echo "unknown")
    
    if [ "$status" = "unhealthy" ]; then
        failures=$((failures + 1))
        log "⚠️  Service unhealthy (attempt $failures/$MAX_FAILURES)"
        echo "$failures" > "$FAILURES_FILE"
        
        if [ "$failures" -ge "$MAX_FAILURES" ]; then
            log "🔄 Service unhealthy. Restarting antigravity-server on $VPS_HOST..."
            ssh "$VPS_HOST" "sudo systemctl restart antigravity-server" && log "✅ Service restarted" || log "❌ Restart failed"
            echo "0" > "$FAILURES_FILE"
        fi
    elif [ "$status" = "degraded" ]; then
        log "⚠️  Service degraded but operational (status: $status)"
        # Don't increment failures for degraded state, just log
    else
        # Healthy
        if [ "$failures" -gt 0 ]; then
            log "✅ Service recovered (was failing $failures times)"
        fi
        echo "0" > "$FAILURES_FILE"
    fi
fi

exit 0

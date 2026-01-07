#!/usr/bin/env bash
# Antigravity Metrics Quick View
# Simple metrics display without complex calculations

set -euo pipefail

VPS_HOST="vps-production"
METRICS_URL="http://localhost:9101/metrics"

echo "Fetching metrics from $VPS_HOST..."
METRICS=$(ssh "$VPS_HOST" "curl -s $METRICS_URL" 2>/dev/null)

echo ""
echo "══════════════════════════════════════════════════════"
echo " Antigravity Manager - Production Metrics"  
echo " Generated: $(date)"
echo "══════════════════════════════════════════════════════"

echo ""
echo "📊 REQUEST STATISTICS"
echo "$METRICS" | grep "^antigravity_requests_total"

echo ""
echo "👥 ACCOUNTS"
echo "$METRICS" | grep "^antigravity_accounts"

echo ""
echo "⏱️  UPTIME"
echo "$METRICS" | grep "^antigravity_uptime_seconds"

echo ""
echo "📝 LOGS"
echo "$METRICS" | grep "^antigravity_log"

echo ""
echo "✅ Analysis complete"

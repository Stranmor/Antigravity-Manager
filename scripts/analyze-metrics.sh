#!/usr/bin/env bash
# Antigravity Metrics Analysis Dashboard
# Fetches and analyzes production metrics from VPS

set -euo pipefail

VPS_HOST="vps-production"
METRICS_URL="http://localhost:9101/metrics"
OUTPUT_FILE="${1:-/tmp/antigravity-metrics-$(date +%Y%m%d-%H%M%S).txt}"

log() {
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

header() {
    echo ""
    echo "═══════════════════════════════════════════════════════════════"
    echo " $*"
    echo "═══════════════════════════════════════════════════════════════"
}

fetch_metrics() {
    ssh "$VPS_HOST" "curl -s $METRICS_URL" 2>/dev/null || {
        log "❌ Failed to fetch metrics from $VPS_HOST"
        exit 1
    }
}

parse_gauge() {
    local metric_name="$1"
    grep "^${metric_name} " | awk '{print $2}' | head -1
}

parse_histogram() {
    local metric_name="$1"
    local quantile="$2"
    grep "^${metric_name}{.*quantile=\"${quantile}\"" | awk '{print $2}' | head -1
}

parse_counter() {
    local metric_name="$1"
    grep "^${metric_name}" | awk '{sum+=$2} END {print sum}'
}

log "Fetching metrics from $VPS_HOST..."
METRICS=$(fetch_metrics)

header "Antigravity Manager - Production Metrics Analysis"
echo "Generated: $(date)"
echo "VPS: $VPS_HOST"

header "📊 Request Statistics"
total_requests=$(echo "$METRICS" | parse_counter "antigravity_requests_total")
success_requests=$(echo "$METRICS" | grep 'antigravity_requests_total{status="200"}' | awk '{print $2}' || echo 0)
error_requests=$(echo "$METRICS" | grep 'antigravity_requests_total{status=~"4..|5.."}' | awk '{sum+=$2} END {print sum}' || echo 0)

echo "Total Requests:    ${total_requests:-0}"
echo "Successful (200):  ${success_requests:-0}"
echo "Errors (4xx/5xx):  ${error_requests:-0}"

if [ "${total_requests:-0}" -gt 0 ]; then
    success_rate=$(awk "BEGIN {printf \"%.2f\", (${success_requests:-0} / ${total_requests}) * 100}")
    echo "Success Rate:      ${success_rate}%"
fi

header "⏱️  Latency Distribution"
echo "Request Duration Buckets:"
echo "$METRICS" | grep "antigravity_request_duration_seconds_bucket" | grep -v "le=\"+Inf\"" | while read -r line; do
    bucket=$(echo "$line" | grep -o 'le="[^"]*"' | cut -d'"' -f2)
    count=$(echo "$line" | awk '{print $2}')
    echo "  ≤ ${bucket}s: $count requests"
done | tail -5

latency_sum=$(echo "$METRICS" | grep "antigravity_request_duration_seconds_sum" | awk '{print $2}' | head -1)
latency_count=$(echo "$METRICS" | grep "antigravity_request_duration_seconds_count" | awk '{print $2}' | head -1)

if [ -n "$latency_sum" ] && [ -n "$latency_count" ] && [ "$latency_count" != "0" ]; then
    avg_latency=$(awk "BEGIN {printf \"%.2f\", $latency_sum / $latency_count}")
    echo "Average Latency:   ${avg_latency}s"
fi

header "👥 Account Pool Status"
accounts_total=$(echo "$METRICS" | parse_gauge "antigravity_accounts_total")
accounts_available=$(echo "$METRICS" | parse_gauge "antigravity_accounts_available")

echo "Total Accounts:    ${accounts_total:-0}"
echo "Available:         ${accounts_available:-0}"

if [ "${accounts_total:-0}" -gt 0 ]; then
    accounts_unavailable=$((accounts_total - accounts_available))
    echo "Unavailable:       $accounts_unavailable"
fi

if [ "${accounts_total:-0}" -gt 0 ] && [ "${accounts_available:-0}" -eq 0 ]; then
    echo "❌ CRITICAL: All accounts unavailable!"
elif [ "${accounts_available:-1}" -lt 2 ]; then
    echo "⚠️  WARNING: Low available accounts (< 2)"
else
    echo "✅ Sufficient accounts available"
fi

header "🔌 Circuit Breaker Status"
circuit_open=$(echo "$METRICS" | grep 'antigravity_circuit_breaker_state{.*state="open"' | wc -l | tr -d ' ')
circuit_half_open=$(echo "$METRICS" | grep 'antigravity_circuit_breaker_state{.*state="half_open"' | wc -l | tr -d ' ')
circuit_closed=$(echo "$METRICS" | grep 'antigravity_circuit_breaker_state{.*state="closed"' | wc -l | tr -d ' ')

echo "Closed (Healthy):  ${circuit_closed:-0}"
echo "Open (Failing):    ${circuit_open:-0}"
echo "Half-Open (Test):  ${circuit_half_open:-0}"

if [ "${circuit_open:-0}" -gt 0 ]; then
    echo "⚠️  WARNING: $circuit_open circuit breaker(s) open"
    echo "$METRICS" | grep 'antigravity_circuit_breaker_state{.*state="open"'
fi

header "🚦 Rate Limiting"
rate_limit_events=$(echo "$METRICS" | parse_counter "antigravity_rate_limit_events_total")

echo "Rate Limit Events: ${rate_limit_events:-0}"

rate_limit_wait_sum=$(echo "$METRICS" | grep "antigravity_rate_limit_wait_seconds_sum" | awk '{print $2}' | head -1)
rate_limit_wait_count=$(echo "$METRICS" | grep "antigravity_rate_limit_wait_seconds_count" | awk '{print $2}' | head -1)

if [ -n "$rate_limit_wait_sum" ] && [ -n "$rate_limit_wait_count" ] && [ "$rate_limit_wait_count" != "0" ]; then
    avg_wait=$(awk "BEGIN {printf \"%.2f\", $rate_limit_wait_sum / $rate_limit_wait_count}")
    echo "Average Wait Time: ${avg_wait}s"
    
    if (( $(echo "$avg_wait > 10" | bc -l) )); then
        echo "⚠️  WARNING: High average wait time (> 10s)"
    fi
fi

header "📝 Log Rotation"
log_files=$(echo "$METRICS" | parse_gauge "antigravity_log_files_total")
log_disk_bytes=$(echo "$METRICS" | parse_gauge "antigravity_log_disk_bytes")
log_rotations=$(echo "$METRICS" | parse_counter "antigravity_log_rotations_total")

if [ -n "$log_disk_bytes" ]; then
    log_disk_mb=$(awk "BEGIN {printf \"%.2f\", $log_disk_bytes / 1024 / 1024}")
    echo "Log Files:         ${log_files:-0}"
    echo "Disk Usage:        ${log_disk_mb} MB"
    echo "Total Rotations:   ${log_rotations:-0}"
fi

header "🎯 Adaptive Rate Limiting (AIMD)"
aimd_rewards=$(echo "$METRICS" | parse_counter "antigravity_aimd_rewards_total")
aimd_penalties=$(echo "$METRICS" | parse_counter "antigravity_aimd_penalties_total")
adaptive_probes=$(echo "$METRICS" | parse_counter "antigravity_adaptive_probes_total")

echo "AIMD Rewards:      ${aimd_rewards:-0}"
echo "AIMD Penalties:    ${aimd_penalties:-0}"
echo "Adaptive Probes:   ${adaptive_probes:-0}"

if [ "${aimd_rewards:-0}" -gt 0 ] || [ "${aimd_penalties:-0}" -gt 0 ]; then
    reward_ratio=$(awk "BEGIN {printf \"%.2f\", ${aimd_rewards:-0} / (${aimd_rewards:-0} + ${aimd_penalties:-0} + 0.001)}")
    echo "Reward Ratio:      ${reward_ratio} (target: > 0.7)"
    
    if (( $(echo "$reward_ratio < 0.5" | bc -l) )); then
        echo "⚠️  WARNING: Low reward ratio - frequent 429 errors"
    fi
fi

header "🎲 Request Hedging"
hedged_requests=$(echo "$METRICS" | parse_counter "antigravity_hedged_requests_total")
hedge_wins=$(echo "$METRICS" | parse_counter "antigravity_hedge_wins_total")
primary_wins=$(echo "$METRICS" | parse_counter "antigravity_primary_wins_total")

echo "Hedged Requests:   ${hedged_requests:-0}"
echo "Hedge Wins:        ${hedge_wins:-0}"
echo "Primary Wins:      ${primary_wins:-0}"

if [ "${hedged_requests:-0}" -gt 0 ]; then
    hedge_win_rate=$(awk "BEGIN {printf \"%.2f\", ${hedge_wins:-0} / ${hedged_requests} * 100}")
    echo "Hedge Win Rate:    ${hedge_win_rate}%"
    
    if (( $(echo "$hedge_win_rate > 30" | bc -l) )); then
        echo "⚠️  NOTICE: High hedge win rate (>30%) - primary requests slow"
    fi
fi

header "💡 Recommendations"

recommendations=()

if [ -n "$avg_latency" ]; then
    if (( $(echo "$avg_latency > 10" | bc -l) )); then
        recommendations+=("🔴 CRITICAL: High average latency (${avg_latency}s > 10s)")
        recommendations+=("  - Check circuit breaker for failing accounts")
        recommendations+=("  - Identify slow upstream accounts via logs")
        recommendations+=("  - Consider disabling underperforming accounts")
    elif (( $(echo "$avg_latency > 5" | bc -l) )); then
        recommendations+=("🟡 WARNING: Average latency degraded (${avg_latency}s > 5s)")
        recommendations+=("  - Monitor for increasing trend")
        recommendations+=("  - Review request hedging effectiveness")
    fi
fi

if [ "${accounts_available:-1}" -lt 2 ]; then
    recommendations+=("🔴 CRITICAL: Low available accounts (${accounts_available:-0}/${accounts_total:-0})")
    recommendations+=("  - Add more accounts via Admin API")
    recommendations+=("  - Check for expired tokens: journalctl | grep '401'")
    recommendations+=("  - Review rate limit events")
fi

if [ "$circuit_open" -gt 0 ]; then
    recommendations+=("🟡 WARNING: ${circuit_open} circuit breaker(s) open")
    recommendations+=("  - Check logs for recurring errors")
    recommendations+=("  - Consider increasing failure threshold")
fi

if [ -n "${avg_wait:-}" ] && (( $(echo "$avg_wait > 10" | bc -l) )); then
    recommendations+=("🟡 WARNING: High rate limit wait times (${avg_wait}s avg)")
    recommendations+=("  - AIMD adaptive limits may be too aggressive")
    recommendations+=("  - Check antigravity_aimd_penalties_total for frequent 429s")
fi

if [ "${#recommendations[@]}" -eq 0 ]; then
    echo "✅ All metrics healthy - no actions required"
else
    for rec in "${recommendations[@]}"; do
        echo "$rec"
    done
fi

header "📁 Output"
echo "Full report saved to: $OUTPUT_FILE"

cat > "$OUTPUT_FILE" <<EOF
═══════════════════════════════════════════════════════════════
 Antigravity Manager - Production Metrics (Full Report)
═══════════════════════════════════════════════════════════════
Generated: $(date)

Raw Metrics (antigravity_* only):
$(echo "$METRICS" | grep "^antigravity_")
EOF

log "✅ Analysis complete - report saved to $OUTPUT_FILE"

# Adaptive Rate Limiting

Antigravity implements **proactive rate limit avoidance** using AIMD (Additive Increase, Multiplicative Decrease) algorithm inspired by TCP congestion control.

## Problem Statement

Traditional rate limit handling is **reactive**:
1. Send request → Get 429 → Mark account limited → Retry on another
2. This causes 200-500ms latency per 429 hit

**Goal:** Predict and avoid 429 BEFORE it happens. Zero additional latency.

## Solution Overview

Three techniques from distributed systems:

1. **AIMD Controller** (TCP Congestion Control) - Adaptive limit discovery
2. **Cheap Probing** - Low-cost limit verification
3. **Speculative Hedging** (Google Spanner) - Parallel probing for calibration

## How It Works

### Per-Account Limit Tracking

Each account maintains:
- `confirmed_limit` - Last limit confirmed via 429 response
- `working_threshold` - 85% of confirmed (safety margin)
- `requests_this_minute` - Current usage counter
- `ceiling` - Historical maximum observed

### AIMD Algorithm

```
On Success (above 70% threshold):
  limit = limit * 1.05  (additive increase: +5%)

On 429 Error:
  limit = limit * 0.7   (multiplicative decrease: -30%)
  confirmed_limit = current_usage
```

### Probe Strategy Selection

Based on usage ratio (`requests / working_threshold`):

| Usage Ratio | Strategy | Description |
|-------------|----------|-------------|
| < 70% | None | Safe zone, no probing needed |
| 70-85% | CheapProbe | Fire-and-forget 1-token probe |
| 85-95% | DelayedHedge | Hedge after P95 latency |
| > 95% | ImmediateHedge | Parallel requests immediately |

### Cheap Probe

A minimal request to test rate limits:
```json
{
  "model": "same-as-original",
  "messages": [{"role": "user", "content": "."}],
  "max_tokens": 1,
  "stream": false
}
```

Cost: ~1 token. Runs in background (fire-and-forget).

- If probe succeeds → AIMD reward (expand limit)
- If probe gets 429 → AIMD penalty (contract limit)

## Configuration

In `gui_config.json`:

```json
{
  "proxy": {
    "adaptive_rate_limit": {
      "enabled": true,
      "safety_margin": 0.85,
      "aimd_increase": 0.05,
      "aimd_decrease": 0.7,
      "probe_threshold_low": 0.70,
      "probe_threshold_high": 0.85,
      "hedge_threshold": 0.95,
      "p95_latency_ms": 2500,
      "persistence_decay_hours": 6,
      "min_limit": 10,
      "max_limit": 1000
    }
  }
}
```

### Configuration Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `enabled` | `true` | Enable adaptive rate limiting |
| `safety_margin` | `0.85` | Working threshold = confirmed × margin |
| `aimd_increase` | `0.05` | Additive increase on success (+5%) |
| `aimd_decrease` | `0.7` | Multiplicative decrease on 429 (×0.7) |
| `probe_threshold_low` | `0.70` | Start cheap probing at 70% usage |
| `probe_threshold_high` | `0.85` | Switch to hedge probing at 85% |
| `hedge_threshold` | `0.95` | Immediate hedging at 95% |
| `p95_latency_ms` | `2500` | Delay before hedge request (P95 LLM latency) |
| `persistence_decay_hours` | `6` | Decay stored limits after N hours |
| `min_limit` | `10` | Floor for learned limits (RPM) |
| `max_limit` | `1000` | Ceiling for learned limits (RPM) |

## Persistence

Learned limits are persisted to SQLite and restored on startup:

```sql
-- Stored per account
INSERT INTO adaptive_limits (account_id, confirmed_limit, ceiling, last_calibration)
VALUES ('abc123', 60, 100, 1704672000);
```

### Decay on Load

Limits decay based on age:
| Age | Confidence |
|-----|------------|
| 0-1 hours | 100% |
| 2-6 hours | 90% |
| 7-24 hours | 70% |
| > 24 hours | 50% |

## Prometheus Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `antigravity_adaptive_probes_total{strategy}` | Counter | Probes by strategy (none/cheap/delayed/immediate) |
| `antigravity_aimd_rewards_total` | Counter | Limit expansions |
| `antigravity_aimd_penalties_total` | Counter | Limit contractions |
| `antigravity_predicted_limit_gauge{account}` | Gauge | Current working threshold |
| `antigravity_hedge_wins_total{winner}` | Counter | Primary vs hedge wins |

## Edge Cases

| Case | Handling |
|------|----------|
| First request (no calibration) | Use conservative default (15 RPM) |
| All accounts at limit | Return error immediately (fail-fast) |
| Limit increased overnight | Probe detects → AIMD expands |
| Limit decreased | 429 → AIMD contracts immediately |
| Stale persisted data | Decay by age, recalibrate quickly |
| Hedging both 429 | Both penalized, return error |

## Algorithm Flow

```
Request arrives
    │
    ▼
┌─────────────────────────────────────────┐
│ 1. Get usage_ratio for selected account │
│    usage = requests_this_minute          │
│    ratio = usage / working_threshold     │
└─────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│ 2. Select probe strategy                │
│    < 70%  → None (safe)                 │
│    70-85% → CheapProbe (background)     │
│    85-95% → DelayedHedge (P95 wait)     │
│    > 95%  → ImmediateHedge              │
└─────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│ 3. Execute request                      │
│    + Fire cheap probe if enabled        │
└─────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│ 4. Handle result                        │
│    Success → AIMD reward (if probing)   │
│    429 → AIMD penalize + rotate account │
└─────────────────────────────────────────┘
```

## Files

| File | Description |
|------|-------------|
| `src/proxy/adaptive_limit.rs` | AdaptiveLimitTracker, AIMD controller |
| `src/proxy/smart_prober.rs` | SmartProber, probe strategies |
| `src/proxy/handlers/helpers.rs` | `maybe_fire_cheap_probe()`, `record_success_with_probe()` |
| `src/proxy/config.rs` | AdaptiveRateLimitConfig |

## Success Criteria

- [x] Zero 429-induced latency after calibration
- [x] <10% quota overhead from probing
- [x] Limits adapt within 1 minute of change
- [x] Persisted limits survive restart with decay
- [x] All metrics exposed to Prometheus

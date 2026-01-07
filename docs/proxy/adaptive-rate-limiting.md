# Adaptive Rate Limiting Configuration

> AIMD-based predictive rate limiting to eliminate 429 latency.

## Overview

The adaptive rate limiting system uses **AIMD (Additive Increase, Multiplicative Decrease)** — the same algorithm that powers TCP congestion control — to predict and avoid rate limit errors before they happen.

**Key Benefits:**
- **Zero 429 latency** after calibration
- **Automatic limit discovery** per account
- **Persistent learned limits** across restarts
- **Speculative hedging** for near-limit requests

## How It Works

```
┌─────────────────────────────────────────────────────────────┐
│                    ADAPTIVE RATE LIMITING                    │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Request arrives                                            │
│       │                                                     │
│       ▼                                                     │
│  ┌─────────────────┐                                        │
│  │ Get usage_ratio │  usage = requests_this_minute          │
│  │ for account     │  ratio = usage / working_threshold     │
│  └────────┬────────┘                                        │
│           │                                                 │
│           ▼                                                 │
│  ┌─────────────────────────────────────────┐                │
│  │ ratio < 85%  → Normal request           │                │
│  │ ratio ≥ 85%  → Skip account, try next   │                │
│  │ ratio = 100% → Account at limit         │                │
│  └─────────────────────────────────────────┘                │
│                                                             │
│  On Success → AIMD reward (+5%)                             │
│  On 429     → AIMD penalty (×0.7)                           │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

## Configuration

Add to `gui_config.json`:

```json
{
  "proxy": {
    "adaptive_rate_limit": {
      "enabled": true,
      "safety_margin": 0.85,
      "aimd_increase": 0.05,
      "aimd_decrease": 0.7,
      "min_limit": 10,
      "max_limit": 1000,
      "enable_cheap_probes": true,
      "enable_hedging": true,
      "p95_latency_ms": 2500,
      "jitter_percent": 0.2,
      "persistence_decay_hours": 6
    }
  }
}
```

## Configuration Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `false` | Enable adaptive rate limiting |
| `safety_margin` | float | `0.85` | Fraction of confirmed limit to use (15% buffer) |
| `aimd_increase` | float | `0.05` | Additive increase on success (+5%) |
| `aimd_decrease` | float | `0.7` | Multiplicative decrease on 429 (×0.7 = -30%) |
| `min_limit` | u64 | `10` | Minimum RPM floor |
| `max_limit` | u64 | `1000` | Maximum RPM ceiling |
| `enable_cheap_probes` | bool | `true` | Send 1-token probes for limit discovery |
| `enable_hedging` | bool | `true` | Enable speculative parallel requests |
| `p95_latency_ms` | u64 | `2500` | P95 latency for hedge delay (ms) |
| `jitter_percent` | float | `0.2` | ±20% jitter on hedge delay |
| `persistence_decay_hours` | u64 | `6` | Age-based decay for persisted limits |

---

## AIMD Algorithm

The system uses **Additive Increase, Multiplicative Decrease** from TCP Reno:

### On Success (above threshold)
```
new_limit = current_limit × (1 + aimd_increase)
          = current_limit × 1.05
          = +5% increase
```

### On 429 Error
```
new_limit = current_limit × aimd_decrease
          = current_limit × 0.7
          = -30% decrease (aggressive backoff)
```

### Convergence
The algorithm converges to the true rate limit through:
1. **Additive increase** slowly probes upward
2. **Multiplicative decrease** rapidly backs off on errors
3. **Sawtooth pattern** oscillates around the true limit

---

## Probe Strategies

Based on current usage ratio, the system selects different strategies:

| Usage Ratio | Strategy | Description |
|-------------|----------|-------------|
| < 70% | **None** | Safe zone, normal requests |
| 70-85% | **CheapProbe** | Background 1-token probe to test limits |
| 85-95% | **DelayedHedge** | Fire backup request after P95 delay |
| > 95% | **ImmediateHedge** | Fire both requests immediately |

### Cheap Probes

Low-cost requests to test rate limits without consuming quota:

```json
{
  "model": "gemini-2.5-flash",
  "messages": [{"role": "user", "content": "."}],
  "max_tokens": 1
}
```

- **Cost:** ~0.001 tokens
- **Purpose:** Verify limit without real request cost
- **Frequency:** Only when approaching threshold (70-85%)

### Speculative Hedging

When near the limit, fire a backup request on a different account:

```
Primary Request (Account A) ─────────────────────────▶ Response
                                    │
                                    │ After P95 delay (2.5s)
                                    ▼
Hedge Request (Account B) ───────────────────────────▶ (canceled if primary wins)
```

- **Winner:** First response to complete
- **Loser:** Canceled via `tokio::select!`
- **Benefit:** Eliminates tail latency from rate limiting

---

## Persistence

Learned limits are persisted to SQLite and survive restarts:

```sql
-- Stored in adaptive_limits table
CREATE TABLE adaptive_limits (
    account_id TEXT PRIMARY KEY,
    confirmed_limit INTEGER NOT NULL,
    ceiling INTEGER NOT NULL,
    last_calibration INTEGER NOT NULL
);
```

### Age-Based Decay

Older limits are less trustworthy (API limits may have changed):

| Age | Confidence | Effective Limit |
|-----|------------|-----------------|
| 0-1 hours | 100% | confirmed_limit × 1.0 |
| 2-6 hours | 90% | confirmed_limit × 0.9 |
| 7-24 hours | 70% | confirmed_limit × 0.7 |
| > 24 hours | 50% | confirmed_limit × 0.5 |

---

## Prometheus Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `antigravity_adaptive_probes_total` | Counter | Total probe requests by strategy |
| `antigravity_aimd_rewards_total` | Counter | Limit increases (successes) |
| `antigravity_aimd_penalties_total` | Counter | Limit decreases (429 errors) |
| `antigravity_hedge_wins_total` | Counter | Hedges that beat primary |
| `antigravity_predicted_limit_gauge` | Gauge | Current working_threshold per account |

### Grafana Queries

```promql
# AIMD activity rate
rate(antigravity_aimd_rewards_total[5m]) + rate(antigravity_aimd_penalties_total[5m])

# Hedge win rate
antigravity_hedge_wins_total / antigravity_hedged_requests_total

# Probe strategy distribution
sum by (strategy) (rate(antigravity_adaptive_probes_total[5m]))
```

---

## Tuning Guidelines

### Conservative (Fewer 429s, Higher Latency)
```json
{
  "safety_margin": 0.70,
  "aimd_increase": 0.02,
  "aimd_decrease": 0.5
}
```

### Aggressive (Lower Latency, More 429s Initially)
```json
{
  "safety_margin": 0.95,
  "aimd_increase": 0.10,
  "aimd_decrease": 0.8
}
```

### High-Throughput Production
```json
{
  "safety_margin": 0.85,
  "aimd_increase": 0.05,
  "aimd_decrease": 0.7,
  "enable_cheap_probes": true,
  "enable_hedging": true,
  "p95_latency_ms": 3000
}
```

---

## Hot Reload

Configuration can be reloaded without restart:

```bash
curl -X POST \
  -H "Authorization: Bearer $API_KEY" \
  http://localhost:9101/api/config/reload
```

The `adaptive_rate_limit` section is hot-reloadable.

---

## Troubleshooting

### Accounts being skipped too aggressively

**Symptom:** All accounts show "at limit" even when quota is available.

**Solution:** Increase `safety_margin` or decrease `persistence_decay_hours`:
```json
{
  "safety_margin": 0.70,
  "persistence_decay_hours": 1
}
```

### Too many 429s during calibration

**Symptom:** Many 429 errors when system is first learning limits.

**Solution:** Start with lower `min_limit` and let AIMD discover true limit:
```json
{
  "min_limit": 5,
  "aimd_decrease": 0.5
}
```

### Hedging consuming too many requests

**Symptom:** Double the expected API usage.

**Solution:** Disable hedging or increase the hedge threshold:
```json
{
  "enable_hedging": false
}
```

---

## Architecture

### Components

1. **AdaptiveLimitTracker** - Per-account limit tracking
2. **AIMDController** - Reward/penalty calculations
3. **SmartProber** - Probe strategy selection and execution
4. **Persistence Layer** - SQLite storage with decay

### Files

| File | Purpose |
|------|---------|
| `src/proxy/adaptive_limit.rs` | Core tracker + AIMD logic |
| `src/proxy/smart_prober.rs` | Probing strategies |
| `src/proxy/config.rs` | AdaptiveRateLimitConfig struct |
| `src/proxy/handlers/helpers.rs` | Handler integration |

---

## References

- [TCP Congestion Control (RFC 5681)](https://tools.ietf.org/html/rfc5681)
- [Google Spanner's Speculative Execution](https://research.google/pubs/pub39966/)
- [AIMD in Practice](https://en.wikipedia.org/wiki/Additive_increase/multiplicative_decrease)

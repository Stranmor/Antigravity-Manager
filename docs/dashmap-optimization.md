# DashMap Contention Optimization

## Current Usage

DashMap is used in several hot paths:

| Component | DashMap Usage | Contention Risk |
|-----------|---------------|-----------------|
| `TokenManager::tokens` | `DashMap<String, Arc<Token>>` | HIGH (read-heavy) |
| `CircuitBreakerManager::states` | `DashMap<String, CircuitState>` | MEDIUM (read-write) |
| `AdaptiveLimitManager::limits` | `DashMap<String, AdaptiveLimitTracker>` | MEDIUM (read-write) |
| `CoalescingManager::pending` | `DashMap<u64, SharedStream>` | LOW (short-lived) |
| `RequestMonitor::active` | `DashMap<String, RequestState>` | LOW (if logging disabled) |

## Contention Patterns

### Read-Heavy (TokenManager)
```rust
// Current: lock on every request
let token = self.tokens.get(&account_id)?;
```

**Problem:** Every request acquires read lock → contention at >1000 RPS

### Read-Write Mix (CircuitBreaker)
```rust
// Pattern 1: Read
let state = self.states.get(&account_id);

// Pattern 2: Read-Modify-Write
self.states.entry(account_id).and_modify(|s| s.failures += 1);
```

**Problem:** Writes block readers in the same shard

### Write-Heavy (Adaptive Limits)
```rust
// Pattern: Increment counter
tracker.requests_this_minute.fetch_add(1, Ordering::Relaxed);
```

**Problem:** False sharing if multiple accounts in same cache line

## Optimization Strategies

### 1. Read-Heavy: Arc + RwLock per Value
```rust
// Before
DashMap<String, Token>

// After
DashMap<String, Arc<RwLock<Token>>>  // Per-value locking

// Or even better
Arc<DashMap<String, Arc<Token>>>  // Immutable values, replace entire Arc
```

**Benefit:** Readers don't block each other within same value

### 2. Shard Tuning
```rust
// Default: 64 shards (good for most cases)
DashMap::with_shard_amount(256)  // Reduce contention for >10k keys
```

**Benefit:** 4x fewer keys per shard → 4x less contention

### 3. Lockless Counters
```rust
// Before
dashmap.entry(key).and_modify(|v| v.counter += 1);

// After
use crossbeam::atomic::AtomicCell;
DashMap<String, AtomicCell<u64>>  // Lock-free increment
```

**Benefit:** Zero lock contention for counters

### 4. Read-Through Cache
```rust
use moka::sync::Cache;

// Layer 1: Lock-free cache
let cache = Cache::builder()
    .max_capacity(1000)
    .time_to_live(Duration::from_secs(60))
    .build();

// Layer 2: DashMap (source of truth)
let tokens = DashMap::new();

// Read path
cache.get_or_insert_with(key, || tokens.get(key).cloned())
```

**Benefit:** 90%+ requests served from lock-free cache

### 5. Batch Writes
```rust
// Before: N writes
for account in accounts {
    dashmap.insert(account.id, account.data);
}

// After: 1 write per shard
let mut batch = HashMap::new();
for account in accounts {
    batch.insert(account.id, account.data);
}
dashmap.extend(batch);  // Locks each shard once
```

**Benefit:** Amortizes lock overhead

## Profiling Tools

### 1. perf (CPU profiling)
```bash
# Record with call graph
sudo perf record -F 99 -g --call-graph dwarf -p $(pidof antigravity-server)

# Analyze contention hotspots
sudo perf report --stdio | grep -A5 'DashMap'
```

### 2. Flamegraph
```bash
# Install
cargo install flamegraph

# Profile
sudo flamegraph -o flamegraph.svg -- ./target/release/antigravity-server serve

# Look for thick DashMap bars (indicates hot path)
```

### 3. Custom Metrics
```rust
// Add to Prometheus metrics
lazy_static! {
    static ref DASHMAP_LOCK_WAIT_SECONDS: Histogram = register_histogram!(
        "antigravity_dashmap_lock_wait_seconds",
        "Time spent waiting for DashMap locks"
    ).unwrap();
}

// Instrument hot paths
let _timer = DASHMAP_LOCK_WAIT_SECONDS.start_timer();
let value = dashmap.get(&key);
```

## Benchmarking

### Synthetic Benchmark
```rust
#[bench]
fn bench_token_lookup_concurrent(b: &mut Bencher) {
    let manager = TokenManager::new();
    // Populate with 1000 accounts
    
    b.iter(|| {
        // Spawn 100 concurrent readers
        let handles: Vec<_> = (0..100)
            .map(|_| {
                tokio::spawn(async {
                    manager.get_available_token().await
                })
            })
            .collect();
        
        // Wait for all
        futures::future::join_all(handles).await;
    });
}
```

### Production Metrics
```promql
# Lock contention indicator
rate(antigravity_dashmap_lock_wait_seconds_sum[5m])
/ rate(antigravity_dashmap_lock_wait_seconds_count[5m])

# Alert if avg wait >1ms
avg(rate(antigravity_dashmap_lock_wait_seconds_sum[5m])
    / rate(antigravity_dashmap_lock_wait_seconds_count[5m])) > 0.001
```

## Implementation Priority

### Phase 1: Measure (Required First)
- [ ] Add lock wait metrics to hot paths
- [ ] Run 24h in production
- [ ] Identify actual bottlenecks (not assumptions)

### Phase 2: Low-Hanging Fruit
- [ ] Tune shard count (if >10k accounts)
- [ ] Replace counters with AtomicCell (lockless)
- [ ] Add moka cache layer for token reads

### Phase 3: Structural Changes
- [ ] Migrate to Arc<RwLock<T>> for read-heavy values
- [ ] Implement batch writes where applicable
- [ ] Consider replacing DashMap with flurry (java-style ConcurrentHashMap)

## Why Deferred

**Current State:**
- VPS traffic: minimal (28s uptime)
- Account count: 6 (DashMap overhead negligible)
- Observed contention: NONE (no metrics yet)

**Premature optimization risks:**
- Adding complexity without evidence
- Introducing bugs in well-tested DashMap code
- Wasting time on non-bottleneck

**When to revisit:**
- Account count >1000
- Request rate >5000 RPS
- Metrics show lock wait >1ms average
- CPU profiling shows DashMap in top 5 hotspots

**Current bottleneck:** Network I/O (upstream API latency ~1-3s), NOT lock contention.

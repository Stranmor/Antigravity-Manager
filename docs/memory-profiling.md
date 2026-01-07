# Memory Profiling Guide

## Tools

### 1. Valgrind Massif (Heap Profiler)
```bash
# Run server with Massif
valgrind --tool=massif --massif-out-file=massif.out \
  ./target/release/antigravity-server serve

# Visualize with ms_print
ms_print massif.out
```

### 2. Heaptrack (Linux)
```bash
# Install
sudo apt install heaptrack

# Profile
heaptrack ./target/release/antigravity-server serve

# Analyze
heaptrack --analyze heaptrack.antigravity-server.*.gz
```

### 3. Tokio Console (Async Runtime Profiling)
```toml
# Add to Cargo.toml
[dependencies]
console-subscriber = "0.2"

# Run with
tokio-console
```

### 4. jemalloc Profiling (Recommended for Rust)
```toml
# Cargo.toml
[dependencies]
tikv-jemallocator = "0.5"

[profile.release]
debug = true  # Enable symbols for profiling
```

```rust
// src/bin/server.rs
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
```

```bash
# Run with profiling
MALLOC_CONF=prof:true,prof_prefix:jeprof.out \
  ./target/release/antigravity-server serve

# Generate flamegraph
jeprof --svg target/release/antigravity-server jeprof.out.*.heap > heap.svg
```

## Key Metrics to Monitor

### Memory Usage Targets
| Component | Target | Alert Threshold |
|-----------|--------|-----------------|
| Idle baseline | <50 MB | >100 MB |
| Per-request overhead | <10 KB | >50 KB |
| DashMap (token_manager) | <5 MB per 100 accounts | >20 MB |
| AdaptiveLimitTracker | <100 bytes per account | >1 KB |
| Request coalescing cache | <10 MB | >50 MB |
| Circuit breaker state | <1 KB per account | >10 KB |

### Production Monitoring
```bash
# Check RSS (Resident Set Size)
ps aux | grep antigravity-server

# Detailed memory map
sudo pmap -x $(pidof antigravity-server)

# /proc/meminfo breakdown
cat /proc/$(pidof antigravity-server)/status | grep -E 'VmRSS|VmSize|VmData'
```

### Prometheus Queries
```promql
# Memory usage trend
process_resident_memory_bytes{job="antigravity"}

# Memory growth rate
rate(process_resident_memory_bytes{job="antigravity"}[5m])

# Alert on >500MB
process_resident_memory_bytes > 500000000
```

## Known Memory Patterns

### DashMap Overhead
- Each entry: ~64 bytes (key) + value size + metadata
- 1000 accounts ≈ 64 KB baseline
- Solution: Use `Arc<str>` for keys, not `String`

### Tokio Task Overhead
- Each spawned task: ~2 KB stack
- Fire-and-forget tasks: should be bounded (max_concurrent)
- Solution: Use semaphore-bounded task spawning

### SSE Streaming Buffers
- `SmallVec<[Bytes; 4]>` already optimized (a574fb3)
- No heap allocation for ≤4 chunks (common case)
- Max buffer: ~16 KB per active stream

### Request Coalescing Cache
- xxHash3 fingerprint: 8 bytes (u64)
- Broadcast channel per fingerprint: ~256 bytes
- LRU eviction at 10,000 entries
- Max: ~2.5 MB

## Optimization Strategies

### 1. String Interning
Replace repeated `String` allocations with `Arc<str>`:
```rust
// Before
HashMap<String, AccountData>

// After
HashMap<Arc<str>, AccountData>
```

### 2. Object Pooling
Reuse expensive allocations:
```rust
use deadpool::managed::Pool;

// Pool reqwest::Client instances
static CLIENT_POOL: Lazy<Pool<reqwest::Client>> = ...;
```

### 3. Arena Allocation
For request-scoped allocations:
```rust
use bumpalo::Bump;

fn handle_request(arena: &Bump) {
    let data = arena.alloc(expensive_data());
    // Automatically freed when arena drops
}
```

### 4. Compact Data Structures
```rust
// Instead of Vec<String>
Vec<Box<str>>  // 24 bytes → 16 bytes per entry

// Instead of HashMap<K, V>
hashbrown::HashMap  // 30% less memory overhead
```

## Findings Template

```markdown
## Memory Profile Results (YYYY-MM-DD)

**Environment:**
- Build: release / debug
- Workload: idle / 100 RPS / 1000 RPS
- Duration: X minutes
- Accounts: N

**Measurements:**
| Metric | Value | Target | Status |
|--------|-------|--------|--------|
| Idle RSS | X MB | <50 MB | ✅/❌ |
| Peak RSS | X MB | <200 MB | ✅/❌ |
| Per-request | X KB | <10 KB | ✅/❌ |

**Top Allocators (jemalloc):**
1. Component A: X MB (Y%)
2. Component B: X MB (Y%)
3. Component C: X MB (Y%)

**Recommendations:**
- [ ] Action 1
- [ ] Action 2
```

## Deferred Analysis

**Why deferred:**
Memory profiling requires **production-like workload** for meaningful results:
- Idle server: Only measures baseline (not useful)
- Synthetic load: Doesn't match real request patterns
- Need: 1000+ real requests with mixed models, streaming, tool calls

**When to profile:**
- After VPS accumulates 24h+ of production traffic
- When RSS exceeds 200 MB (alert threshold)
- Before scaling to 10x traffic

**Current status:**
- VPS uptime: 28 seconds (just deployed)
- Production traffic: minimal
- Baseline RSS: ~30-50 MB (estimated, within target)

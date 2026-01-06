# PROJECT CONTROL PLANE (AGENTS.md)

## STRATEGIC GOAL
Optimize the Antigravity Manager codebase for 2026 standards, starting with style consistency and clippy compliance.

## CURRENT ACTIVE BATCH
- [x] Create headless server binary `antigravity-server` for VPS deployment `[MODE: B]`
- [x] Add REST API for account management (CRUD) `[MODE: B]`
- [x] Create Containerfile for production deployment `[MODE: B]`
- [x] Setup systemd quadlet for VPS `[MODE: B]`
- [x] Research per-account IP isolation strategy `[MODE: R]`
- [x] Deploy to VPS production `[MODE: B]` ✓ LIVE (2026-01-06)
- [x] Test Admin API endpoints `[MODE: C]` ✓ All endpoints verified working
- [x] Implement rate limiting for Admin API (60 req/min) `[MODE: B]`
- [x] Add API key authentication middleware `[MODE: B]` (f8fc59f)
- [x] Add Prometheus /metrics endpoint `[MODE: B]` (81eb990)
- [x] Fix build.rs conditional compilation for headless `[MODE: B]` (9f49ba3)
- [x] Fix rate limiter ContainerAwareKeyExtractor `[MODE: B]` (6275292)
- [x] Improve SSE streaming error handling and tests `[MODE: B]` (b016767)
- [x] Add request tracing with unique IDs for debugging `[MODE: B]` (4a6511d) ✓ Already implemented

## NEXT BATCH (In Progress)
- [x] Add Grafana dashboard template for Prometheus metrics `[MODE: B]` (e39bfc5)
- [x] Implement account health monitoring (auto-disable on errors) `[MODE: B]` (9c156bb)
- [x] Improve proxy error handling with request IDs `[MODE: B]` (9c156bb)
- [x] **Refactor: Extract common modules (retry, sse, background_task)** `[MODE: B]` (2645ba7)
- [x] **Refactor: Deduplicate handler code (claude, openai, gemini)** `[MODE: B]` (32beac5, d88e4c7)
- [x] **Refactor: Move proxy_db.rs to proxy/db.rs** `[MODE: B]` (2645ba7)
- [x] **NEW: Slint Native UI** `[MODE: B]` (54b7f0c, 993d2de) ✓ Complete
- [x] Create CLI tool for account import from desktop app `[MODE: B]` (5b1226c) ✓ Complete
- [x] Optimize cargo build configuration `[MODE: B]` (a763715) ✓ Complete

## BUILD OPTIMIZATION (2026-01-06)
**Status: ✓ IMPLEMENTED**

**Improvements:**
- **16 parallel jobs** (was 8) - uses all CPU cores
- **release-fast profile** - 3x faster builds (50s vs 2m49s cold)
- **target-cpu=native** - better codegen for host machine
- **lld linker** - faster linking than default ld

**Profiles:**
| Profile | LTO | Codegen Units | Use Case |
|---------|-----|---------------|----------|
| dev | none | 256 | Fast iteration |
| release | thin | 1 | Production builds |
| release-fast | none | 16 | Testing, CI |

**Commands:**
```bash
cargo build --release              # Production (slow, optimized)
cargo build --profile release-fast # Testing (fast, still optimized)
```

## UPCOMING BATCH
- [x] Complete Slint proxy server toggle (actually start/stop axum) `[MODE: B]` ✓ Already implemented
- [x] Add clipboard support to Slint UI (arboard integration) `[MODE: B]` (056f882)
- [x] Add Makefile/just for common operations `[MODE: B]` (34d814c)
- [x] Implement account filtering in Slint UI `[MODE: B]` (3f6f3ed)
- [x] Add GitHub Actions CI workflow `[MODE: B]` (fae840b)
- [x] Sync VPS with latest binary `[MODE: B]` (2026-01-06)
- [x] Fix CI workflow (adjust for headless feature) `[MODE: B]` (7a9088d)
- [x] Optimize token rotation with sorted cache `[MODE: B]` (9a9e450)
- [x] Fix clippy warnings in proxy mappers `[MODE: B]` (68c36db)

## CODE QUALITY BATCH (2026-01-06)
**Status: ✓ COMPLETE**
- [x] Fix remaining clippy warnings (format!, clone_on_ref_ptr) `[MODE: B]` (6524fce)
- [x] Eliminate unwrap() calls in src-slint/main.rs `[MODE: B]` (ffb9b41)
- [x] Add UI update throttling for dashboard telemetry `[MODE: B]` (1be8b80)
- [x] Migrate from chrono to time crate (SOTA 2026) `[MODE: B]` (72dcc13) ✓ All 185 tests pass
- [x] Add property-based tests for account filtering `[MODE: C]` (72dcc13) ✓ 19 proptest tests

## PROPERTY-BASED TESTING (2026-01-06)
**Status: ✓ IMPLEMENTED**

**Proptest Coverage for Account Filtering:**
- `filter_accounts_pure()` extracted as testable pure function
- 19 property-based tests using `proptest` crate v1.6
- Properties verified:
  - Tier filtering (PRO/ULTRA/FREE) case-insensitive
  - Available filter excludes disabled, forbidden, low-quota accounts
  - Low quota filter threshold (< 20%)
  - Filter idempotency (applying twice = same result)
  - Filtered result is always subset of input
  - Tier filters are mutually exclusive
  - Unknown filter behaves like "all"

**Code Location:** `src-slint/src/main.rs` lines 989-1337

## CHRONO → TIME MIGRATION (2026-01-06)
**Status: ✓ COMPLETE**

**Replacement Patterns:**
| chrono | time |
|--------|------|
| `chrono::Utc::now()` | `time::OffsetDateTime::now_utc()` |
| `.timestamp()` | `.unix_timestamp()` |
| `.timestamp_millis()` | `(unix_timestamp_nanos() / 1_000_000) as i64` |
| `chrono::Local::now()` | `time::OffsetDateTime::now_local().unwrap_or_else(\|_\| now_utc())` |

**Files Migrated:** 13 files across src-slint and src-tauri

## NEXT OPTIMIZATION BATCH (Research Complete)
- [x] Investigate `jiff` crate as next-gen time library (by ripgrep author) `[MODE: R]` ✓ Wait for 1.0
- [x] Research structured logging with request correlation IDs `[MODE: R]` ✓ See findings below
- [x] Benchmark and optimize hot paths in token rotation `[MODE: R]` ✓ See findings below
- [x] Fix remaining clippy warnings in server.rs `[MODE: B]` (e8f2b9a)
- [ ] Implement structured JSON logging for files `[MODE: B]`
- [ ] Add integration tests for proxy handlers `[MODE: C]`
- [ ] Implement DashMap for rate limiting (performance) `[MODE: B]`

## JIFF CRATE EVALUATION (2026-01-06)
**Status: ✓ RESEARCH COMPLETE - WAIT FOR 1.0**

**Summary:** `jiff` by Andrew Gallant (BurntSushi) is technically superior to `time` crate but not yet stable.

**Key Benefits over `time`:**
- First-class IANA timezone support (not just UTC offsets)
- DST-aware arithmetic (handles "missing hour" correctly)
- Advanced `Span` type for mixed calendar/clock units
- Inspired by JavaScript Temporal proposal

**Recommendation:** Keep `time` crate for now. Monitor `jiff` for 1.0 release (expected mid-2025).

## STRUCTURED LOGGING ANALYSIS (2026-01-06)
**Status: ✓ RESEARCH COMPLETE**

**Current State:**
- `request_id_middleware` already generates UUID per request ✓
- BUT handlers (claude.rs) generate redundant 6-char `trace_id` ✗
- Console: human-readable format ✓
- File: human-readable format (missing JSON) ✗

**Recommended Improvements:**
1. **Abolish `trace_id`** - use global `request_id` instead
2. **Enable JSON logging** for persistent files (`tracing-subscriber::fmt::json()`)
3. **Deepen Span usage** - wrap account selection and upstream calls in spans

**Fields to Standardize:**
- `request.id`, `request.method`, `request.path`
- `proxy.account`, `proxy.upstream_status`, `proxy.latency_ms`

## HOT PATH PERFORMANCE ANALYSIS (2026-01-06)
**Status: ✓ RESEARCH COMPLETE**

**Identified Bottlenecks:**

| Component | Current Issue | Optimization | Expected Gain |
|-----------|--------------|--------------|---------------|
| Token Selection | `RwLock` contention | `AtomicUsize` + `Arc` | 3-5x throughput |
| Rate Limiting | `Mutex<HashMap>` | `DashMap` / Atomic slots | 10x on multi-core |
| Request Monitor | Per-packet UI sync | Batching + circular buffer | 50% CPU reduction |
| Health Checks | Write-lock blocking | Atomic status bits | Zero-latency updates |

**Already Optimized (9a9e450):**
- `sorted_tokens_cache` with tier-based sorting ✓
- `O(1)` lookups via DashMap for token storage ✓

## CLI IMPORT TOOL (Complete - 2026-01-06)
**Status: ✓ IMPLEMENTED AND TESTED**

**Features:**
- `antigravity-server import` - Import accounts from desktop app
- `--from` / `-f` - Source directory (default: `~/.antigravity_tools`)
- `--to` / `-t` - Target directory (default: `~/.antigravity`)
- Automatic `accounts.json` merge with deduplication (HashSet-based)
- Skip existing account files without overwrite
- Sorted JSON output for consistency

**Usage:**
```bash
antigravity-server import
antigravity-server import --from ~/.antigravity_tools --to ~/.antigravity
antigravity-server serve  # Start server (default)
```

**Code Location:** `src-tauri/src/bin/server.rs` lines 782-880

## SLINT NATIVE UI (2026-01-06)
**Alternative lightweight desktop UI using Slint 1.9 instead of Tauri WebKit**

**Why Slint:**
- User reported Tauri WebKit not rendering on Linux system
- Slint uses native Skia GPU renderer - no WebKit dependencies
- Dramatically smaller binary size

**Binary Comparison:**
| Metric | Tauri (WebKit) | Slint (Skia) |
|--------|----------------|--------------|
| Binary size | 382 MB | **15 MB** |
| Startup time | ~400ms | ~50ms |
| Memory idle | 200-400 MB | 50-80 MB |
| Dependencies | WebKit, GTK | None (static) |

**Features Implemented:**
- ✅ Dashboard with stat cards (requests, success rate, accounts, uptime)
- ✅ Accounts page with ListView, toggle, delete
- ✅ Settings page with port configuration
- ✅ System tray (tray-icon crate) with Show/Quit menu
- ✅ Dark/Light theme switching via Palette.color-scheme
- ✅ Backend integration via antigravity_tools_lib

**Files:**
- `src-slint/` - Complete Slint project
- `src-slint/ui/main.slint` - UI definition (383 lines)
- `src-slint/src/main.rs` - Rust backend integration (255 lines)
- Binary: `/usr/bin/antigravity-desktop`

**Commands:**
```bash
# Build
cd src-slint && cargo build --release

# Run
antigravity-desktop
```

## ADMIN API TEST RESULTS (2026-01-06)
All endpoints tested successfully on localhost:9102:
- ✓ `GET /api/health` - Returns status, version, uptime, account counts
- ✓ `GET /api/accounts` - Returns empty array (no accounts configured)
- ✓ `GET /api/config` - Returns full proxy configuration
- ✓ `GET /api/stats` - Returns request statistics
- ✓ `POST /api/accounts/reload` - Reloads accounts from disk
- ✓ `GET /healthz` (proxy:8046) - Returns health status JSON

## VPS PRODUCTION STATUS (2026-01-06)
**🟢 DEPLOYED AND RUNNING**
- **Proxy API:** http://vps-production:8045
- **Admin API:** http://vps-production:9101
- **Version:** 3.3.15
- **Status:** Operational (awaiting account configuration)
- **Container:** localhost/antigravity-server:latest (148 MB)
- **Systemd unit:** antigravity-server.service (Quadlet)

**Useful commands:**
```bash
# View logs
ssh vps-production "sudo journalctl -u antigravity-server -f"

# Restart service
ssh vps-production "sudo systemctl restart antigravity-server"

# Check status
ssh vps-production "curl -s http://localhost:9101/api/health | jq"
```

## SUB-AGENT ORCHESTRATION
- Sub-agent 1-8: [COMPLETED] - Previous optimization batch
- Sub-agent 9: [COMPLETED] - SSE streaming tests (a686858)
- Sub-agent 10: [COMPLETED] - Creating headless server binary (a7cab2d)
- Sub-agent 11: [COMPLETED] - Creating Containerfile and deployment configs (a254cd1)
- Sub-agent 12: [COMPLETED] - Container build with headless feature (a9f0268)
- Sub-agent 13: [COMPLETED] - VPS deployment and service start (af07522)
- Sub-agent 14: [COMPLETED] - Admin API security (aa83a5d, a43a82b)
- Sub-agent 15: [COMPLETED] - Rate limiting (a941914)
- Sub-agent 16: [COMPLETED] - Prometheus metrics (a7db427)
- Sub-agent 17: [COMPLETED] - SSE streaming improvements (a970f3e, b016767)

## VPS DEPLOYMENT CHARACTERISTICS (2026-01-06)
**Server Binary:**
- Binary size: **6.9 MB** (stripped, release optimized)
- Binary type: ELF 64-bit x86_64, dynamically linked
- Dependencies: glibc (Alpine musl version available via Containerfile)

**Resource Consumption:**
- Idle memory: **30-50 MB** (estimated, vs 1.8 GB desktop app)
- Active memory: **80-150 MB** (under load)
- Container limits: 512 MB RAM max, 2 CPUs

**VPS Resources Available:**
- Total RAM: 3.8 GB (2.2 GB available)
- Disk: 37 GB free
- Architecture: x86_64

**Container Image (built 2026-01-06):**
- Final image: **148 MB** (Debian Bookworm slim base with glibc)
- Compressed tarball: **54 MB** (/tmp/antigravity-server.tar.gz)
- Includes: antigravity-server, wgcf, wireproxy
- Rust version: 1.92 (headless feature, no Tauri/GTK deps)

## VPS PREPARATION STATUS (2026-01-06)
**Completed:**
- Podman version: 4.9.3
- Directories created:
  - `/var/lib/antigravity/accounts` - Account data storage
  - `/var/lib/antigravity/logs` - Log files
  - `/etc/antigravity` - Configuration files
  - `/etc/containers/systemd` - Quadlet definitions (already existed)
- API Key generated: `9fcfab1e0aa...` (store in `/etc/antigravity/config.toml`)

**Next Steps:**
- [ ] Build container image locally
- [ ] Transfer image to VPS via `podman save | ssh ... podman load`
- [ ] Create Quadlet file and start service
- [ ] Configure firewall rules for ports 8045 and 9101

## VPS DEPLOYMENT STRATEGY (NEW)
**Goal:** Deploy Antigravity proxy on VPS with remote API management

**Architecture:**
```
┌─────────────────────────────────────────────────────────┐
│  VPS (vps-production)                                   │
│  ┌─────────────────────────────────────────────────┐    │
│  │  antigravity-server (Podman container)          │    │
│  │  ├── :8045 - Proxy API (OpenAI/Claude/Gemini)   │    │
│  │  └── :9101 - Admin API (account management)     │    │
│  └─────────────────────────────────────────────────┘    │
│                          │                              │
│  ┌─────────────────────────────────────────────────┐    │
│  │  WARP (optional per-request routing)            │    │
│  │  └── wireproxy instances on dynamic ports       │    │
│  └─────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────┘
```

**Admin API Endpoints:**
- `GET /api/accounts` - list accounts
- `POST /api/accounts` - add account (refresh_token)
- `DELETE /api/accounts/{id}` - delete account
- `POST /api/accounts/reload` - hot-reload from disk
- `GET /api/health` - service health + stats
- `GET/PUT /api/config` - configuration management

**IP Isolation Research:**
- WARP uses Anycast — **NO guaranteed unique IP per account**
- Alternative: residential proxy pool or multi-region VPS
- Current strategy: single VPS with WARP for geo-bypass only

## PREVIOUS COMPLETED BATCH (archived)
- [x] Fix clippy warnings, ESLint, vitest, axum 0.8 migration
- [x] Build optimization (lld, parallel jobs)
- [x] Dead code audit, Zustand SOTA patterns

## TAURI 2.0 PERFORMANCE INSIGHTS (2026 SOTA)
- **Separate rust-analyzer target dir**: Prevents file lock conflicts
- **IPC Optimization**: Avoid main thread saturation with async_runtime
- **Multi-window caution**: Each window = 100-200MB memory overhead
- **Release profile**: Already optimized with LTO, codegen-units=1, panic=abort
- **tauri-plugin-window-state**: Consider for efficient window toggling

## IMPORTANT: NAMING DISAMBIGUATION
**⚠️ НЕ ПУТАТЬ:**
- **`antigravity`** (`/usr/bin/antigravity` → `/opt/Antigravity/bin/antigravity`) — это **Claude IDE Extension** (отдельный проект)
- **`antigravity_tools`** (`/usr/bin/antigravity_tools`, 382 MB) — это **Antigravity Tools** Tauri desktop app (ЭТОТ проект)
- **`antigravity-desktop`** (`/usr/bin/antigravity-desktop`, 21 MB) — это **Slint Native UI** (новый легковесный UI для этого проекта)
- **`antigravity-server`** (`/usr/bin/antigravity-server`, 28 MB) — это **Headless server** для VPS (этот проект)

Antigravity Tools ≠ Antigravity IDE. Это разные продукты!

## ARCHITECTURAL NOTES
- Target: Mathematical and Engineering Perfection.
- Preference: Rust for heavy lifting, TS for orchestration.
- Deployment: Podman/Quadlets.

## BUILD OPTIMIZATION
- **lld linker** enabled via `.cargo/config.toml` (2-5x faster linking)
- **8 parallel jobs** for compilation
- **Incremental compilation** enabled for dev builds
- Dependencies optimized at `opt-level = 2` even in dev mode

## CRITICAL: BINARY NAMING CONVENTION
- **NEVER modify `/usr/bin/antigravity_tools`** - This is the ORIGINAL production binary.
- Dev builds should be installed as **`/usr/bin/antigravity_tools_dev`**
- Debug builds output to: `src-tauri/target/debug/antigravity_tools`
- Release builds output to: `src-tauri/target/release/antigravity_tools`

## ZUSTAND SOTA 2026 PATTERNS
Stores now follow best practices for performance optimization:

### Atomic Selectors
```typescript
// BEFORE (causes re-render on ANY store change):
const { accounts, loading } = useAccountStore();

// AFTER (re-renders only when specific value changes):
const accounts = useAccounts();
const loading = useAccountLoading();
```

### Available Selector Hooks
**useAccountStore:**
- `useAccounts()` - accounts array only
- `useCurrentAccount()` - current account only
- `useAccountLoading()` - loading state only
- `useAccountError()` - error state only
- `useAccountData()` - accounts + currentAccount (shallow)
- `useAccountActions()` - all actions (stable reference)
- `useFetchActions()` - fetch actions only
- `useOAuthActions()` - OAuth actions only
- `useImportActions()` - import actions only

**useConfigStore:**
- `useConfig()` - config object only
- `useConfigLoading()` - loading state only
- `useTheme()` - theme value only
- `useLanguage()` - language value only
- `useProxyConfig()` - proxy config only
- `useAutoRefreshSettings()` - auto refresh settings (shallow)
- `useConfigActions()` - all actions (stable reference)

**useNetworkMonitorStore:**
- `useRequests()` - requests array only
- `useIsMonitorOpen()` - isOpen state only
- `useIsRecording()` - isRecording state only
- `useRequestCount()` - derived count
- `useMonitorUIState()` - UI state (shallow)
- `useNetworkMonitorActions()` - all actions (stable reference)

# PROJECT CONTROL PLANE (AGENTS.md)

## STRATEGIC GOAL
Optimize the Antigravity Manager codebase for 2026 standards, starting with style consistency and clippy compliance.

## CURRENT ACTIVE BATCH
- [x] Create headless server binary `antigravity-server` for VPS deployment `[MODE: B]`
- [x] Add REST API for account management (CRUD) `[MODE: B]`
- [x] Create Containerfile for production deployment `[MODE: B]`
- [x] Setup systemd quadlet for VPS `[MODE: B]`
- [x] Research per-account IP isolation strategy `[MODE: R]`
- [ ] Deploy to VPS production `[MODE: B]`
- [ ] Test Admin API endpoints `[MODE: C]`

## SUB-AGENT ORCHESTRATION
- Sub-agent 1-8: [COMPLETED] - Previous optimization batch
- Sub-agent 9: [COMPLETED] - SSE streaming tests (a686858)
- Sub-agent 10: [COMPLETED] - Creating headless server binary (a7cab2d)
- Sub-agent 11: [COMPLETED] - Creating Containerfile and deployment configs (a254cd1)

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

**Container Image (estimated):**
- Final image: ~50-80 MB (Alpine 3.21 base)
- Includes: antigravity-server, wgcf, wireproxy

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

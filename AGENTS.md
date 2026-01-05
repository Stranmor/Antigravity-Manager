# PROJECT CONTROL PLANE (AGENTS.md)

## STRATEGIC GOAL
Optimize the Antigravity Manager codebase for 2026 standards, starting with style consistency and clippy compliance.

## CURRENT ACTIVE BATCH
- [x] Fix clippy warnings regarding inline format arguments (variables directly in format string). `[MODE: B]`
- [x] Fix documentation comment style (remove empty lines after `///`). `[MODE: B]`
- [x] Fix ESLint issues in frontend (Restrict template expressions, Floating promises, Unsafe types). `[MODE: B]`
- [x] Resolve `vitest.config.ts` TS parsing error. `[MODE: B]`
- [x] Perform SOTA Scan for Tauri/Vite/Rust dependencies. `[MODE: R]`
- [x] Migrate to axum 0.8 (path syntax: `:param` → `{param}`). `[MODE: B]`
- [x] Add build optimization config (lld linker, parallel jobs). `[MODE: B]`
- [x] Audit for dead code and unused imports. `[MODE: S]`
- [x] Remove dead ProxyError variants (AuthError, ServiceUnavailable, ConfigError, ProxyResult). `[MODE: B]`
- [x] Fix React useEffect anti-pattern in ProxyMonitor.tsx. `[MODE: B]`
- [x] Refactor Zustand stores for SOTA 2026 patterns (atomic selectors, useShallow, action separation). `[MODE: B]`

## SUB-AGENT ORCHESTRATION
- Sub-agent 1: [COMPLETED] - ESLint fixes committed.
- Sub-agent 2: [COMPLETED] - SOTA Scan completed, axum 0.8 migrated.
- Sub-agent 3: [COMPLETED] - Fixed ProxyError dead code warnings (a95edac)
- Sub-agent 4: [COMPLETED] - Dead code audit (a9b2ecd)
- Sub-agent 5: [COMPLETED] - ESLint React anti-patterns fixed (a9b8c47)
- Sub-agent 6: [COMPLETED] - Dead code cleanup committed (af42ab5) - commit c88f0fe
- Sub-agent 7: [RUNNING] - Improve test coverage (a6e6f1f)
- Sub-agent 8: [COMPLETED] - Zustand stores optimized with SOTA 2026 patterns

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

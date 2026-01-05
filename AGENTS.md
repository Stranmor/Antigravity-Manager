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
- [ ] Audit for dead code and unused imports. `[MODE: S]`

## SUB-AGENT ORCHESTRATION
- Sub-agent 1: [COMPLETED] - ESLint fixes committed.
- Sub-agent 2: [COMPLETED] - SOTA Scan completed, axum 0.8 migrated.

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

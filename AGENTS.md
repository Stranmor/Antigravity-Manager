# PROJECT CONTROL PLANE (AGENTS.md)

## STRATEGIC GOAL
Optimize the Antigravity Manager codebase for 2026 standards, starting with style consistency and clippy compliance.

## CURRENT ACTIVE BATCH
- [x] Fix clippy warnings regarding inline format arguments (variables directly in format string). `[MODE: B]`
- [x] Fix documentation comment style (remove empty lines after `///`). `[MODE: B]`
- [ ] Fix ESLint issues in frontend (Restrict template expressions, Floating promises, Unsafe types). `[MODE: B]`
- [ ] Resolve `vitest.config.ts` TS parsing error. `[MODE: B]`
- [ ] Perform SOTA Scan for Tauri/Vite/Rust dependencies to ensure we aren't using "radioactive" (6+ months old) tools. `[MODE: R]`
- [ ] Audit for "Architectural Rage" opportunities (messy imports, dead code). `[MODE: S]`

## SUB-AGENT ORCHESTRATION
- Sub-agent 1: [ACTIVE] - Running `npm run lint -- --fix` and manual TS fixes.
- Sub-agent 2: [ACTIVE] - Performing SOTA Scan on dependencies.

## ARCHITECTURAL NOTES
- Target: Mathematical and Engineering Perfection.
- Preference: Rust for heavy lifting, TS for orchestration.
- Deployment: Podman/Quadlets.

## CRITICAL: BINARY NAMING CONVENTION
- **NEVER modify `/usr/bin/antigravity_tools`** - This is the ORIGINAL production binary.
- Dev builds should be installed as **`/usr/bin/antigravity_tools_dev`**
- Debug builds output to: `src-tauri/target/debug/antigravity_tools`
- Release builds output to: `src-tauri/target/release/antigravity_tools`

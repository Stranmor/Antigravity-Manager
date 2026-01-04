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

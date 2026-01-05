# Changelog

All notable changes to this project will be documented in this file.


## [3.3.16] - 2026-01-05

### Added
  - feat: Configure Vitest and fix error handling tests (unresolved test failures)
  - feat(proxy): improve 429 retry logic with per-model rate limiting and auto session unbind

### Changed
  - chore(deps): update npm packages to latest versions
  - docs: update AGENTS.md with completed tasks and build optimization notes

### Fixed
  - fix(lint): resolve all ESLint errors and warnings
  - fix(lint): resolve more ESLint errors in ApiProxy and ProxyMonitor
  - fix(lint): fix TypeScript build errors in tests and Accounts
  - fix(lint): fix void expression in arrow function shortcuts
  - fix(lint): use wrapper handlers for remaining async callbacks in ApiProxy
  - fix(lint): add wrapper handlers for async functions in ApiProxy
  - fix(lint): continue resolving ESLint errors in ApiProxy.tsx
  - fix(lint): resolve ESLint errors in ApiProxy.tsx
  - fix(lint): resolve ESLint errors in tests, config, and Settings

### Performance
  - perf(build): add release profile for smaller binary size
  - perf(build): add cargo config for faster builds with lld linker

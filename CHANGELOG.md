# Changelog

## v3.3.20 (2026-01-09)
- **Request Timeout Enhancement**: Increased request timeout from 600s to 3600s for long text processing.
- **Auto-Stream Conversion**: Automatically converts non-stream requests to stream requests for Google API, drastically reducing 429 errors.
- **macOS Dock Icon Fix**: Resolved issue where clicking Dock icon wouldn't reopen window on macOS.

## v3.3.19 (2026-01-09)
- **Model Routing Refactoring**: Simplified model routing with wildcard matching and automatic migration of old rules.
- **Model-Level Rate Limiting**: Implemented rate limiting at the model level to prevent one model's quota exhaustion from affecting others.
- **Optimistic Reset Strategy**: Added a dual-layer defense for 429 errors with buffered delays and optimistic resets.

## v3.3.18 (2026-01-08)
- **Smart Rate Limit Optimization**: Implemented intelligent exponential backoff and real-time quota refreshing for 429 errors.
- **Model Routing Center Bug Fix**: Fixed GroupedSelect Portal event handling and added missing internationalization translations.
- **macOS Old Version Compatibility Fix**: Replaced `<dialog>` tag to fix "add account" dialog not showing on older macOS.
- **Built-in Passthrough Model Routing Fix**: Ensured built-in passthrough models are not incorrectly subject to family mapping rules.

## v3.3.17 (2026-01-08)
- **OpenAI Protocol Thinking Display Enhancement**: Added `reasoning_content` field support for Gemini 3 thinking models in OpenAI compatible format.
- **FastMCP Framework Compatibility Fix**: Resolved `anyOf`/`oneOf` type loss in JSON Schema generation.
- **Frontend UI/UX Optimization**: API proxy routing refactoring, persistent account view mode, and improved table layout.
- **Custom Grouped Select Component**: Custom component to replace native `<select>` for better cross-platform consistency and dark mode support.
- **Internationalization Improvement**: Expanded `en.json` and removed hardcoded strings for better multi-language support.
- **Antigravity Identity Injection**: Implemented intelligent Antigravity identity injection across Claude, OpenAI, and Gemini protocols.

## v3.3.16 (2026-01-07)
- **Performance Optimization**: Refactored account quota refresh to run concurrently, significantly improving speed.
- **UI Visual Design Optimization**: Improved API proxy page visuals, enhanced dark mode, and fixed theme switching animations.
- **Stability & Tool Fixes**: Fixed Grep/Glob parameter issues, `RedactedThinking` errors, JSON Schema cleaning, and 400 auto-retry.
- **High Concurrency Performance Optimization**: Resolved UND_ERR_SOCKET errors in high concurrency scenarios by optimizing lock contention and removing blocking waits.
- **Log System Optimization**: Optimized log levels and added automatic cleanup for old log files.
- **Gemini 3 Pro Thinking Model Fix**: Fixed 404 errors for `gemini-3-pro-high` and `gemini-3-pro-low` models.
- **Internet Access Functionality Downgrade Optimization**: Ensured all models downgrade to `gemini-2.5-flash` when internet access is enabled, due to `googleSearch` tool support.
- **OpenAI Protocol Multi-Candidate Support**: Added support for `n` parameter and multi-candidate SSE responses.
- **Internet Search Functionality Enhancement**: Improved internet search source display with Markdown citation format.
- **MCP Tool Enum Value Type Fix**: Resolved 400 errors for MCP tools due to enum values being numbers or booleans.
- **Response Body Log Limit Optimization**: Increased response body log limit to 10MB to prevent truncation of image generation responses.
- **Audio Transcription API Support**: Added `/v1/audio/transcriptions` endpoint compatible with OpenAI Whisper API.
- **Linux System Compatibility Enhancement**: Fixed transparent window rendering on Linux by disabling DMA-BUF renderer.
- **Monitoring Middleware Capacity Optimization**: Increased monitoring middleware payload limit to 100MB.
- **Installation and Distribution Optimization**: Homebrew Cask now supports Linux installations.
- **API Monitoring Enhancement**: API monitoring logs now display account email and model mapping.

## v3.3.15 (2026-01-04)
- **Claude Protocol Compatibility Enhancement**:
    - Fixed Opus 4.5 first request errors and function call signature validation.
    - Improved `cache_control` cleaning and tool parameter remapping.
    - Added configurable safety settings and `effort` parameter support.
    - Optimized retry jitter and signature capture.

## v3.3.14 (2026-01-03)
- **Claude Protocol Robustness Improvement**: Enhanced Thinking Block signature validation and tool/function call compatibility.
- **SSE Parsing Error Recovery Mechanism**: Implemented error monitoring and graceful degradation for streaming responses.
- **Dashboard Statistics Fix**: Fixed incorrect low quota statistics for disabled accounts.

## v3.3.13 (2026-01-03)
- **Thinking Mode Stability Fixes**: Fixed empty Thinking content errors, smart downgrade validation errors, and model switching signature errors.
- **Account Rotation Rate Limit Mechanism Optimization**: Fixed `quotaResetDelay` parsing and distinguished between `QUOTA_EXHAUSTED` and `RATE_LIMIT_EXCEEDED` rate limit reasons.

## v3.3.12 (2026-01-02)
- **Critical Fixes**: Fixed Antigravity Thinking Signature error by disabling fake Thinking block injection and removing fake signature fallback.

## v3.3.11 (2026-01-02)
- **Critical Fixes**:
    - **Cherry Studio Compatibility Fix (Gemini 3)**: Removed mandatory Prompt injection for Coding Agent and user message suffix for Gemini 3 models.
    - **Gemini 3 Python Client Crash Fix**: Removed forced `maxOutputTokens: 64000` for Gemini requests.
- **Core Optimization**: Unified backoff strategy system for different error types.
- **Scoop Installation Compatibility Support**: Added Antigravity startup parameter configuration and intelligent database path detection.
- **Browser Environment CORS Support Optimization**: Clarified HTTP method list and optimized preflight cache.
- **Account Table Drag and Drop Sorting**: Added drag and drop sorting for account tables with persistent storage.

## v3.3.10 (2026-01-01)
- **Upstream Endpoint Fallback Mechanism**: Implemented multi-endpoint fallback strategy for increased service availability.
- **Log System Optimization**: Refactored log levels, filtered heartbeat requests, and reduced log volume.
- **Imagen 3 Image Generation Enhancement**: Added support for -2k resolution and -21x9 aspect ratio.
- **Model Detection API**: Provided `POST /v1/models/detect` interface for real-time detection of image generation capabilities.
- **Background Task Downgrade Model Fix**: Corrected background task downgrade model to `gemini-2.5-flash-lite`.
- **Account Active Disabling Function**: Added active disabling of accounts, affecting only the proxy pool.
- **UI Experience Improvement**: Unified alert/confirm pop-ups and fixed Tooltip occlusion issues.

## v3.3.9 (2026-01-01)
- **Full Protocol Scheduling Alignment**: `Scheduling Mode` now covers OpenAI, Gemini native, and Claude protocols.
- **Industrial-Grade Session Fingerprint**: Upgraded SHA256 content hashing for sticky session IDs.
- **Accurate Rate Limiting and 5xx Fault Avoidance**: Integrated Google API JSON parsing for `quotaResetDelay` and 5xx fault isolation.
- **Intelligent Scheduling Algorithm Upgrade**: `TokenManager` actively avoids rate-limited or isolated accounts.
- **Global Rate Limit Synchronization**: Cross-protocol rate limit tracker for real-time synchronization.
- **Claude Multi-Modal Completion**: Fixed 400 errors for Claude CLI when transmitting PDF documents.

## v3.3.8 (2025-12-31)
- **Proxy Monitoring Module**: Real-time request tracking, persistent log storage, and advanced filtering.
- **UI Optimization and Layout Improvements**: Unified Toggle styles and optimized layout density.
- **Zai Dispatcher Integration**: Multi-level dispatch modes (Exclusive, Pooled, Fallback) and built-in MCP service support.
- **Automatic Account Anomaly Handling**: Automatically disables accounts with invalid OAuth refresh tokens.
- **Dynamic Model List API**: Real-time synchronization of built-in and custom model mappings.
- **Account Quota Management and Model Tiered Routing**: Optimized background task handling, concurrent locks, and account ranking.

## v3.3.7 (2025-12-30)
- **Proxy Core Stability Fixes**: JSON Schema hardening, enhanced background task robustness, and improved thought signature capture.

## v3.3.6 (2025-12-30)
- **OpenAI Image Functionality Deep Adaptation**: Full support for `/v1/images/generations`, `/v1/images/edits`, and `/v1/images/variations` endpoints.

## v3.3.5 (2025-12-29)
- **Core Fixes and Stability Enhancements**: Fixed Claude Extended Thinking 400 errors, added 429 error automatic account rotation, and maintained unit tests.
- **Log System Optimization**: Cleaned redundant logs and added local timezone support.
- **UI Optimization**: Optimized account quota refresh time display.

## v3.3.4 (2025-12-29)
- **OpenAI/Codex Compatibility Enhancement**: Fixed image recognition, Gemini 400 error handling, and protocol stability.
- **Linux Build Strategy Adjustment**: Reverted to Ubuntu 22.04 for compilation due to resource issues.

## v3.3.3 (2025-12-29)
- **Account Management Enhancement**: Added subscription level recognition, multi-dimensional filtering, and UI/UX optimization.
- **Core Fixes**: Fixed Claude Extended Thinking 400 errors due to missing `thought: true` tag.

## v3.3.2 (2025-12-29)
- **New Features**: Claude Protocol internet search citation support and Thinking mode stability enhancement (Global Signature Store v2).
- **Optimizations & Bug Fixes**: Enhanced data model robustness and optimized SSE conversion engine.

## v3.3.1 (2025-12-28)
- **Critical Fixes**: Claude Protocol 400 errors, full protocol built-in internet access tool adaptation, and client dirty data purification.
- **Core Optimization & Token Saving**: Full link tracking, closed-loop audit logs, and Claude CLI background task intelligent "hijacking" (Token Saver).
- **Stability Enhancement**: Fixed Rust compilation and test case errors due to model field definition updates.

## v3.3.0 (2025-12-27)
- **Major Updates**: Codex CLI & Claude CLI deep adaptation, OpenAI protocol stack refactoring, and privacy-first network binding control.
- **Frontend Experience Upgrade**: Visualized multi-protocol endpoints.

## v3.2.8 (2025-12-26)
- **Bug Fixes**: OpenAI protocol multi-modal and image model support fixes.

## v3.2.7 (2025-12-26)
- **New Features**: Auto-start on boot and account list pagination size selector.
- **Bug Fixes**: JSON Schema cleaning logic enhancement, custom database import fix, and proxy stability with image generation performance optimization.

## v3.2.6 (2025-12-26)
- **Critical Fixes**: Claude Protocol deep optimization (Claude Code experience enhancement), test suite hardening.

## v3.2.3 (2025-12-25)
- **Core Enhancements**: Process management architecture optimization with precise path identification and self-exclusion.
- **UX Improvements**: Path configuration UI and multi-language adaptation.

## v3.2.2 (2025-12-25)
- **Core Updates**: Full log persistence system upgrade, Project ID acquisition logic enhancement, and Google free quota intelligent routing (Token Saver).
- **Bug Fixes**: Claude Thought Chain Signature validation fix, Gemini model mapping error fix.

## v3.2.1 (2025-12-25)
- **New Features**: Custom DB import, Project ID real-time synchronization, and OpenAI & Gemini protocol enhancements.
- **Bug Fixes**: OpenAI custom mapping 404 fix, Linux process management optimization, OpenAI protocol adaptation fix, and Claude Thought Chain validation error fix.

## v3.2.0 (2025-12-24)
- **Core Architecture Refactor**: Rewritten proxy engine, optimized Linux process management.
- **GUI Interaction Revolution**: Reworked dashboard with average quota monitoring and "best account recommendation" algorithm.
- **Account Management Enhancement**: Support for JSON/regex batch import of tokens, optimized OAuth authorization process.
- **Protocol and Routing Expansion**: Native support for OpenAI, Anthropic (Claude Code) protocols; new "Model Routing Center".
- **Multi-Modal Optimization**: Deep adaptation for Imagen 3, support for 100MB ultra-large payloads.
- **Installation Experience Optimization**: Formal support for Homebrew Cask installation; built-in macOS "app corruption" automated troubleshooting guide.
- **Global Upstream Proxy**: Unified management of internal and external network requests, supporting HTTP/SOCKS5 protocols and hot reloading.

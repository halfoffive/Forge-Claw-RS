## Summary

This PR addresses the audit findings documented in `AUDIT_REPORT.md` (109 findings across 6 perspectives). It iteratively fixes P0/P1 issues and brings the full CI matrix to green.

### What Changed

- **Phase 1 (Blocking P0)**
  - Fixed `web/index.html` scaffold
  - Corrected fictional dependency versions in `web/package.json`
  - Integrated `rust-embed` into `forgeclaw-server` to serve the WebUI bundle
  - Fixed CI build order so the Rust job depends on the frontend artifact

- **Phase 2 (Security P0)**
  - `ShellTool` now clears the environment and strips sensitive variables
  - Config file is written with `0o600` permissions
  - Web server defaults to `127.0.0.1`; weak/default tokens are rejected
  - WebSocket auth switched from reusable `?token=` to single-use 60s tickets
  - Expanded dangerous-command blacklist and fixed bypass patterns
  - Cross-user session takeover via WebSocket is blocked

- **Phase 3 (Concurrency / Orchestrator P0)**
  - `SessionData.history` is now `Arc<RwLock<History>>` with in-place appends
  - `run_turn` enforces a default maximum of 25 turns
  - Tool errors are fed back into LLM history
  - LLM stream errors return `OrchestratorEvent::Error` instead of empty `Complete`

- **Phase 4 (WebUI P0)**
  - Added 5 core views: Chat, Sessions, Prompts, Tools, Settings
  - Added lazy-loaded routes with auth guards
  - Added unified API client and Pinia stores
  - Added navigation layout in `App.vue`

- **Phase 5 (P1 Long Tail)**
  - WebSocket heartbeat, disconnect abort, and panic handling
  - Constant-time token comparison and login rate limiting
  - Generic 500 responses with detailed server-side logging
  - Request body limits, CORS whitelist, and middleware ordering
  - `Role` enum, `UserPublic`, and `secrecy::SecretString` for token safety
  - `PromptEngine` concurrency optimization
  - CI hardening (`concurrency`, `timeout-minutes`, `permissions`)

### Verification

- `cargo fmt --all -- --check` ✅
- `cargo clippy --workspace --all-targets -- -D warnings` ✅
- `cargo test --workspace` ✅ (142 tests)
- `pnpm install && pnpm build && pnpm typecheck` ✅

Two rounds of multi-angle sub-agent review were performed; the only follow-up fixes were frontend type alignment and the addition of `GET /api/auth/ticket` so the chat view can obtain a fresh single-use WebSocket ticket.

### Related

- Spec: `.trae/specs/iterative-bug-fix-and-pr/spec.md`
- Tasks: `.trae/specs/iterative-bug-fix-and-pr/tasks.md`
- Checklist: `.trae/specs/iterative-bug-fix-and-pr/checklist.md`
- Audit Report: `AUDIT_REPORT.md`

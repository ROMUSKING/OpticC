# ASYNC REPO & PROMPT PROTOCOL

This repository uses the checked-in source tree plus `jules_prompts/` as the live shared-memory layer for agentic work.

## Core workflow
1. Read `README.md`, `Cargo.toml`, `docs/kernel-and-performance-roadmap.md`, and the prompt file for your area.
2. Make the smallest verified change that advances the current goal.
3. Update the matching prompt file when verified status, blockers, setup, or ownership changes.
4. Record only current-session facts; do not preserve stale pass counts or superseded blockers.
5. Hand off clearly to the next owning area.

## Current repo-wide priorities (2026-04-21)
1. **SQLite truth gate first**: the real GitHub-hosted SQLite archive now downloads, preprocesses, compiles, and links through OpticC, but the smoke test still fails with unresolved `u8` and `vtabCallConstructor`.
2. **Kernel work continues after SQLite smoke is green**: atomic builtins, freestanding flags, GCC/Kbuild compatibility, and the validation ladder remain the main kernel path.
3. **Use the GitHub SQLite archive as the standard large-input source** unless a session intentionally pins a different local truth file.

## Shared-state rules
- `docs/kernel-and-performance-roadmap.md` is the canonical high-level status document.
- `jules_prompts/*.md` hold area-specific lessons, blockers, and next actions.
- If a document is stale, update it instead of layering contradictory notes elsewhere.

## Verified environment notes
- `cargo build` succeeds.
- `cargo test` succeeds.
- `cargo test -- --list | grep -c ': test'` reports **405** discovered tests.
- The integration harness now supports remote archive download in the normal build setup via `curl`/`wget` fallback; the Cargo `network` feature is no longer required for that path.

## Handoff discipline
- Put backend/runtime blockers in the owning prompt file.
- Keep setup instructions concrete and copy-pastable.
- Prefer current verified commands over aspirational instructions.

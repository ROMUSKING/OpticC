# OpticC Orientation (Current Session Baseline)

## What this repository is
OpticC is a Rust C99-to-LLVM compiler with a preprocessor, typed backend, build pipeline, SQLite integration harness, benchmark suites, and a growing kernel-oriented direct-driver mode.

## Canonical docs to read first
1. `docs/kernel-and-performance-roadmap.md` — current goals and blockers
2. `jules_prompts/00_protocol.md` — workflow and shared-memory rules
3. `jules_prompts/01_orchestrator.md` — milestone ownership and sequencing
4. `jules_prompts/09_integration.md` — SQLite verification instructions
5. `README.md` — user-facing commands and setup

## Current verified baseline (2026-04-21)
- `cargo build` succeeds.
- `cargo test` succeeds; `cargo test -- --list | grep -c ': test'` reports **405** discovered tests.
- The SQLite fixture pipeline still passes via `cargo test test_full_pipeline_local_fixture -- --nocapture`.
- A real GitHub-hosted SQLite archive now works through the integration harness default flow:
  - download ✅
  - preprocess ✅
  - OpticC compile ✅
  - shared-library link ✅
  - smoke test ❌
- Current real-archive smoke blocker from `/tmp/optic_sqlite_github/out/integration_report.md`:
  - undefined reference to `u8`
  - undefined reference to `vtabCallConstructor`

## Current goals
### Phase A — SQLite truth gate
Use the GitHub-hosted SQLite amalgamation as the standing large-input truth source. The immediate objective is to turn the current compile/link success into a passing smoke test.

### Phase B — Linux kernel readiness
After the SQLite truth gate is green, the next priorities remain:
1. atomic builtins (`__sync_*`, `__atomic_*`)
2. remaining freestanding/kernel-driver behavior
3. progressive validation (coreutils → module → kernel subtree → tinyconfig/QEMU)

## Current setup quickstart
```bash
apt-get update && apt-get install -y build-essential clang llvm llvm-dev lld binutils unzip curl
cargo build
cargo test

# default integration target now points at a GitHub SQLite archive
cargo run -- integration-test \
  --test-dir /tmp/optic_sqlite_github/test \
  --output-dir /tmp/optic_sqlite_github/out
```

## Practical notes for future sessions
- Prefer `docs/kernel-and-performance-roadmap.md` over stale historical totals in older notes.
- The integration harness no longer depends on Cargo's `network` feature for remote archives; it can fall back to `curl`/`wget`.
- Treat the SQLite smoke failure as a codegen/runtime blocker, not as a download/setup blocker.
- Keep prompt files in `jules_prompts/` current whenever verified status changes.

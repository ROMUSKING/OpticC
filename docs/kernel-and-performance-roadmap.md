# OpticC Kernel and Performance Roadmap

## Current verified baseline (2026-04-22)
- `cargo build` succeeds.
- `cargo test` succeeds.
- `cargo test -- --list | grep -c ': test'` reports **405** discovered tests in the current workspace.
- The SQLite local-fixture integration test still passes end to end.
- A real GitHub-hosted SQLite amalgamation archive now passes:
  - download
  - extraction
  - preprocessing
  - OpticC compilation
  - shared-library link
- The same real-archive run still fails the smoke test on runtime correctness.
- Latest diagnostics: `sqlite3_open(":memory:")` path reaches `openDatabase` and crashes in error cleanup (`sqlite3_free`) with an invalid pointer (`0x313ffff`).

## Phase A — SQLite truth gate
### Goal
Turn the verified large-input compile/link pipeline into a passing SQLite smoke test and benchmark gate.

### Current state
- Acquisition is no longer blocked: the integration harness now defaults to a GitHub SQLite archive and can fetch it without the Cargo `network` feature.
- Build-scale compilation is working for a real SQLite amalgamation archive.
- Runtime/smoke correctness is still blocked by pointer-semantics/codegen issues in SQLite error paths.

### Immediate next actions
1. Fix remaining pointer-semantics/runtime corruption in real-SQLite smoke (`openDatabase` / URI-error path).
2. Re-run `cargo run -- integration-test` against the GitHub archive after each relevant backend/type-system fix.
3. Keep `benchmark --suite sqlite --sqlite-source ...` aligned with the same truth source once smoke is green.

### Exit criteria
- `cargo run -- integration-test` passes end to end against the GitHub-hosted SQLite archive.
- The smoke binary can open `:memory:`, create a table, execute SQL, and close cleanly.
- SQLite benchmarks run against the same truth source without correctness regressions.

## Phase B — Linux kernel readiness
### Goal
Advance from SQLite-scale validation to kernel-relevant frontend and codegen compatibility.

### Priorities
1. atomic builtins (`__sync_*`, `__atomic_*`)
2. remaining freestanding/kernel driver behavior
3. GCC/Kbuild compatibility refinements
4. progressive validation beyond SQLite

## Phase C — Progressive validation ladder
1. SQLite truth gate
2. coreutils/busybox-scale targets
3. out-of-tree kernel module
4. kernel subtree
5. `tinyconfig` build
6. QEMU boot verification

## Agent ownership
- **Jules-Integration**: SQLite truth-gate reruns and smoke validation
- **Jules-Backend-LLVM**: unresolved symbol/codegen investigation
- **Jules-Type-System**: type-resolution issues exposed by real SQLite
- **Jules-Benchmark**: SQLite performance/correctness benchmarking once smoke is green
- **Jules-Kernel-Compilation / Jules-Build-System**: post-SQLite kernel validation ladder

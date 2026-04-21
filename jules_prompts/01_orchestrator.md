# Jules-Orchestrator

Maintain this file as the live orchestration brief for OpticC.

## Current milestone framing (2026-04-21)
### Phase 1 — Core compiler infrastructure
Implemented.

### Phase 2 — SQLite truth gate
**In progress.**

Verified now:
- local SQLite fixture integration passes
- real GitHub-hosted SQLite archive download/extract/preprocess/compile/link passes
- real-archive smoke test still fails with unresolved `u8` and `vtabCallConstructor`

This means SQLite is no longer blocked on acquisition or coarse-scale compilation. The remaining work is semantic/runtime correctness in the produced library.

### Phase 3 — Linux kernel readiness
Still the strategic target, but SQLite smoke correctness remains the active gate before escalating validation breadth.

## Immediate orchestration priorities
1. Route the current real-SQLite smoke failure to the backend/type-system owners.
2. Keep integration/docs aligned on the GitHub SQLite archive default flow.
3. Resume kernel-priority work after SQLite smoke is green:
   - atomic builtins
   - freestanding/kernel flags
   - GCC/Kbuild compatibility
   - progressive validation ladder

## Ownership map
- `09_integration.md` — SQLite truth-gate execution and reporting
- `07_backend_llvm.md` — unresolved codegen/runtime behavior
- `11_type_system.md` — real-world type-resolution gaps
- `16_kernel_compilation.md` — post-SQLite kernel ladder
- `17_cli_compatibility.md` / `14_build_system.md` — driver and Kbuild behavior

## Orchestrator rules
- Prefer `docs/kernel-and-performance-roadmap.md` as the canonical status summary.
- Do not call Phase 2 complete while the real-archive smoke test is still red.
- Avoid stale numeric claims unless re-verified in the current session.

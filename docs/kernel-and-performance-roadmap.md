# OpticC Kernel and Performance Roadmap

## Current Verified Baseline

- 378 repository tests pass.
- SQLite-oriented benchmark suites are available through the CLI.
- Warm recompilation is now measured separately from cold compilation.
- OpticC uses a persistent object cache to accelerate unchanged rebuilds.
- SQLite integration accepts local source inputs in addition to remote archives.
- Kernel-style direct-driver smoke builds now produce valid ELF object files and depfiles.
- Preprocessor feature probes for __has_attribute, __has_builtin, and __has_include are now verified.

## Phase A — SQLite Truth Gate

### Goal
Compile SQLite-scale inputs reliably and benchmark them against GCC and Clang.

### Exit Criteria
- `benchmark --suite sqlite` runs on bundled SQLite-style stress cases.
- `benchmark --suite sqlite --sqlite-source /path/to/sqlite3.c` benchmarks a real local SQLite source file.
- `benchmark --suite rebuild` reports cold compile and warm recompile times separately.
- Shared-library output works for SQLite integration paths.

### Agent Ownership
- Jules-Benchmark: benchmark harness and reporting
- Jules-Build-System: cache, PIC, linking
- Jules-Integration: SQLite verification gate

## Phase B — Linux Kernel Frontend Readiness

### Goal
Reach semantic compatibility for the features the kernel depends on most heavily.

### Priorities
1. Atomic builtins: `__sync_*`, `__atomic_*`
2. Attributes: packed, noinline, always_inline, constructors
3. Type system gaps: flexible arrays, anonymous structs/unions, `_Static_assert`
4. Preprocessor feature probes: `__has_attribute`, `__has_builtin`, `__has_include`, `__VA_OPT__`

### Agent Ownership
- Jules-GNU-Extensions
- Jules-Type-System
- Jules-Preprocessor
- Jules-Backend-LLVM

## Phase C — Freestanding and Kbuild Integration

### Goal
Make OpticC usable as a realistic drop-in kernel compiler driver.

### Deliverables
- `-ffreestanding`, `-mcmodel=kernel`, `-mno-red-zone`
- GCC-style argument compatibility
- depfile and response-file support
- `CC=optic_c` compatibility for targeted kernel builds

### Agent Ownership
- Jules-Kernel-Compilation
- Jules-Build-System
- Jules-CLI-Compatibility

## Phase D — Progressive Validation Ladder

1. Coreutils subset
2. Small SQLite and library-heavy projects
3. Out-of-tree kernel module
4. Kernel library subtree
5. `tinyconfig` kernel build
6. QEMU boot verification

## Phase E — Beyond Kernel: Performance Leadership

### Strategy
Compete where OpticC can be structurally stronger:
- faster warm recompilation through cache-aware object reuse,
- better compiler diagnostics for large C codebases,
- targeted optimization tuned for systems software,
- incremental benchmark dashboards that track cold and warm builds separately.

### Key Metrics
- cold compile time
- warm recompile time
- cache effectiveness
- binary size
- correctness on validation ladder targets

## Current Sprint Status

### Completed in this step
- GCC-style direct compiler-driver entry now accepts common kernel and Makefile invocations.
- Version and target probes now work for build-system discovery.
- Minimal dependency-file generation is available for incremental build flows.
- Simple CC-style Makefile compilation has been verified end-to-end.

### Immediate Next Actions
1. Advance from stubbed smoke builds to a real out-of-tree kernel module validation.
2. Harden freestanding semantics beyond flag acceptance.
3. Expand SQLite benchmarking to real local amalgamation runs in CI.
4. Add more kernel-oriented attributes and type-system gaps.
5. Add optimization passes and compare warm rebuild speedups to GCC and Clang.

### Newly Verified Progress
- Representative M7 atomic lowering is now live and verified through generated LLVM IR.
- Verified instructions now include atomic read-modify-write, compare-and-swap, and sequentially consistent fences.
- M10 preprocessor feature probes now answer kernel-style checks for __has_attribute, __has_builtin, and __has_include.
- End-to-end kernel-style probe compilation now succeeds with direct-driver flags, depfile generation, and feature-gated source.

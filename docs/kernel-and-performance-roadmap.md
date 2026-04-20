# OpticC Kernel and Performance Roadmap

## Current Verified Baseline

- 394 repository tests pass.
- SQLite-oriented benchmark suites are available through the CLI.
- Warm recompilation is now measured separately from cold compilation.
- OpticC uses a persistent object cache to accelerate unchanged rebuilds.
- SQLite integration accepts local source inputs in addition to remote archives.
- Kernel-style direct-driver smoke builds now produce valid ELF object files and depfiles.
- The kernel header work tree is now installed in-container at /lib/modules/$(uname -r)/build.
- Force-included headers are now honored in kernel-style direct-driver flows.
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
2. Attributes: builtin and freestanding follow-up after verified packed, noinline, always_inline, and constructor/destructor support
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
4. Finish anonymous struct and union promotion semantics and continue objtool-safe kernel lowering.
5. Add optimization passes and compare warm rebuild speedups to GCC and Clang.

### Newly Verified Progress
- Representative M7 atomic lowering is now live and verified through generated LLVM IR.
- Verified instructions now include atomic read-modify-write, compare-and-swap, and sequentially consistent fences.
- M8 now includes verified support for packed structs, noinline, always_inline, hot, constructor/destructor lowering, and compile-time GNU builtins for type compatibility and choose-expression flows.
- Packed tagged declarations such as struct __attribute__((packed)) S now parse correctly and produce 5-byte layout in regression coverage.
- Flexible array headers such as struct Flex { int len; char data[]; } now lower with the correct 4-byte header size in regression coverage.
- Constructor and destructor attributes now emit llvm.global_ctors and llvm.global_dtors entries with regression coverage.
- M10 preprocessor feature probes now answer kernel-style checks for __has_attribute, __has_builtin, and __has_include.
- End-to-end kernel-style probe compilation now succeeds with direct-driver flags, depfile generation, feature-gated source.
- Real out-of-tree module builds now progress past header discovery and objtool validation with the installed kernel work tree.
- The current live kernel blocker is narrowed to module metadata survival at modpost, specifically missing MODULE_LICENSE emission.

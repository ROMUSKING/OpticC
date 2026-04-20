# OpticC Kernel and Performance Roadmap

## Current Verified Baseline

- 394 repository tests pass.
- SQLite 3.49.2 (89K preprocessed lines) compiles through full pipeline: OpticC → .ll → .bc → .o (no panics, no verification errors).
- `llvm-as` and `llc -filetype=obj` succeed on OpticC-generated IR after minimal post-processing.
- Linker undefined references reduced from 725 to 33 (95.4% symbol resolution).
- Function deduplication eliminates all LLVM `.NNNN` suffixed names.
- Indirect calls via `build_indirect_call` for function pointers through expressions.
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
Compile SQLite-scale inputs reliably, link cleanly, pass sanity tests, and benchmark against GCC/Clang.

### Current Status (2026-04-20)
- ✅ Compilation: 89K lines → 119K lines LLVM IR (no panics)
- ✅ Verification: `llvm-as` + `llc` produce valid .o file
- ⚠️ Linking: 33 undefined references remain (function pointer params, va_list functions)
- ❌ Runtime: segfault in openDatabase (semantic correctness in codegen)

### Remaining Work (Priority Order)
1. **P0: Function pointer parameters as call arguments** — `xDel`, `xDestroy`, etc. (22 refs). Root cause: when a function pointer param like `void(*xDel)(void*)` is used as `func(ptr, xDel)`, the backend resolves `xDel` via `self.functions` hash (auto-declared) instead of `self.variables` (parameter alloca). Fix: in `lower_ident`, prioritize `self.variables` lookup (already does), but the param isn't in variables because `lower_func_def` doesn't insert function pointer params. Fix param type detection to recognize function pointer declarators.
2. **P0: va_list functions not compiled** — `sqlite3VMPrintf`, `sqlite3_str_vappendf` (8 refs). Root cause: parser/backend doesn't handle `va_list` as a typedef'd type for parameters. The function definition exists in source but gets skipped because the parameter type resolution fails.
3. **P1: Indirect call through struct member** — `sqlite3Config.m.xFree(p)` generates `@0` (now fixed with indirect call fallback). Remaining: calls where field name is resolved as func_name (e.g., `xCallback`). Need parser-level fix to distinguish `obj.field(args)` from `func(args)`.
4. **P1: Control flow correctness** — Dead code after terminators, branches to undefined labels (handled by post-processing but should be fixed in compiler). Key issues: switch case body placement, goto targets.
5. **P2: Global variable initialization** — Complex initializers (struct literals, array initializers with designators).
6. **P2: Type casting correctness** — Implicit conversions at call sites and assignments.

### Exit Criteria
- `optic_c compile sqlite3_preprocessed.c` → .o links with zero undefined references
- SQLite sanity test (open :memory:, CREATE TABLE, INSERT, SELECT) passes
- `benchmark --suite sqlite --sqlite-source` runs successfully
- Performance within 2x of GCC -O0 for compile time

### Agent Ownership
- Jules-Backend-LLVM: function pointer codegen, param registration, indirect calls
- Jules-Parser: function pointer declarator recognition, va_list type handling  
- Jules-Type-System: function pointer types, va_list typedef resolution
- Jules-Integration: SQLite verification gate
- Jules-Benchmark: benchmark harness and reporting

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

## Current Sprint Status (2026-04-20)

### Completed
- SQLite 3.49.2 compiles to valid ELF .o through OpticC (119K IR lines, 608KB object)
- Function deduplication: `get_function()` before `add_function()` eliminates 1304 duplicate declarations
- Indirect call support: `build_indirect_call` for function pointer callees
- `__builtin_unknown` → no-op (prevents 52 invalid call emissions)
- Dead code removal, undefined label fix, entry block fix (post-processing)
- GCC-style direct compiler-driver entry accepts common kernel and Makefile invocations
- Version and target probes work for build-system discovery
- Minimal dependency-file generation available for incremental build flows
- Simple CC-style Makefile compilation verified end-to-end

### Immediate Next Actions
1. **Fix function pointer parameter registration** in `lower_func_def` — function pointer params must be stored in `self.variables` with ptr type
2. **Fix va_list typedef resolution** — parser must resolve `va_list` / `__builtin_va_list` as a known type so functions with va_list params compile
3. **Fix member access call codegen** — `obj.field(args)` must lower as indirect call through loaded field pointer, not direct call to `@field`
4. **Implement atomic builtins** (M7) — `__sync_*` and `__atomic_*` → LLVM `atomicrmw`/`cmpxchg`
5. **Advance kernel module compilation** — out-of-tree module reaching modpost
6. **Add PIC/PIE support** — `-fPIC` flag for shared library output
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

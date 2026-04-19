# LLVM Optimization Pass Integration for OpticC

This document tracks LLVM optimization pass pipeline integration and optimization-level behavior.

## PROMPT MAINTENANCE REQUIREMENT
Maintain this file as the live reference for optimization pass status. After wiring up a new pass or changing optimization behavior, update this prompt so later agents inherit the current status.

## CONTEXT
The Linux kernel is normally compiled with `-O2`. OpticC must produce correct, functional code at this optimization level. Currently, the `-O` flag is passed through to LLVM but the inkwell 0.9 PassManager API is effectively a no-op. Proper pass pipeline integration is needed.

## PASS PIPELINE BY OPTIMIZATION LEVEL

### -O0 (No Optimization)
- No optimization passes
- Generate straightforward IR: alloca for every variable, load/store for every access
- Useful for debugging; currently the effective behavior

### -O1 (Basic Optimization)
- `mem2reg` — Promote alloca to SSA registers (most impactful single pass)
- `simplifycfg` — Simplify control flow graph (merge blocks, remove unreachable)
- `early-cse` — Common subexpression elimination
- `instcombine` — Algebraic simplification of instructions

### -O2 (Standard Optimization — Kernel Default)
All of -O1, plus:
- `gvn` — Global value numbering
- `loop-simplify` + `licm` — Loop invariant code motion
- `loop-unroll` — Unroll small loops
- `sccp` — Sparse conditional constant propagation
- `dse` — Dead store elimination
- `adce` — Aggressive dead code elimination
- `inline` — Function inlining (unless `-fno-inline`)
- `tailcallelim` — Tail call elimination (unless `-fno-optimize-sibling-calls`)
- `reassociate` — Expression reassociation

### -O3 (Aggressive Optimization)
All of -O2, plus:
- More aggressive inlining thresholds
- `loop-vectorize` — Auto-vectorization
- `slp-vectorize` — Superword-level parallelism
- `argpromotion` — Promote pointer arguments to scalars
- `loop-unswitch` — Unswitching of loops

### -Os (Optimize for Size)
Similar to -O2 but:
- Inlining thresholds reduced
- No loop unrolling
- Prefer smaller code sequences
- `-Os` maps to LLVM `OptimizationLevel::Os`

### -Oz (Minimize Size)
More aggressive than -Os:
- Minimal inlining
- Aggressive code size reduction
- `-Oz` maps to LLVM `OptimizationLevel::Oz`

## INKWELL PASS MANAGER STATUS

### Current State
- inkwell 0.9's `PassManager::run_on()` is effectively a no-op
- The `-O` flag is accepted by the CLI but has no effect on generated code
- All code is effectively generated at -O0 quality

### Approach Options

#### Option A: LLVM New Pass Manager via C API
Use the LLVM C API directly (`LLVMRunPasses`) to invoke the new pass manager:
```rust
// Pseudocode
extern "C" {
    fn LLVMRunPasses(
        module: LLVMModuleRef,
        passes: *const c_char,
        target_machine: LLVMTargetMachineRef,
        options: LLVMPassBuilderOptionsRef,
    ) -> LLVMErrorRef;
}
// passes = "default<O2>" for standard optimization
```
This is the preferred approach for LLVM 18+.

#### Option B: llc Optimization
Pass `-O2` to `llc` when generating object files:
```bash
llc -O2 -filetype=obj input.ll -o output.o
```
This applies LLVM's optimization pipeline during machine code generation.
Currently used in the build pipeline.

#### Option C: opt Tool
Run `opt` separately on the generated .ll file:
```bash
opt -O2 input.ll -o optimized.ll
llc -filetype=obj optimized.ll -o output.o
```
This is the simplest but adds an extra process invocation.

### Recommended Strategy
1. **Immediate**: Use Option B (llc -O2) for correct kernel builds — already partially in place
2. **Medium term**: Implement Option A for in-process optimization
3. **Long term**: Upgrade inkwell or use llvm-sys directly for full pass manager control

## KERNEL REQUIREMENTS

### Critical
- `-O2` must produce **correct** code — correctness over performance
- No miscompilations of atomic operations, volatile accesses, or inline asm
- `volatile` loads/stores must not be optimized away
- Inline asm side effects must be preserved
- Function calls with `__attribute__((noinline))` must not be inlined

### Important
- Dead code elimination should work (kernel uses `__attribute__((unused))`)
- Constant folding for `sizeof`, `offsetof`, compile-time expressions
- Basic register allocation quality (avoid excessive spills)

### Nice to Have
- Link-time optimization (LTO) — kernel LTO is optional
- Profile-guided optimization (PGO) — not needed for first boot
- Auto-vectorization — not needed for tinyconfig

## FLAG INTERACTIONS

| Flag | Effect on Passes |
|------|-----------------|
| `-fno-inline` | Disable the `inline` pass |
| `-fno-optimize-sibling-calls` | Disable `tailcallelim` pass |
| `-fno-strict-aliasing` | Disable TBAA metadata (no type-based alias analysis) |
| `-fno-delete-null-pointer-checks` | Preserve null checks (kernel needs this) |
| `-fno-common` | No common symbols — affects global variable lowering |
| `-fno-asynchronous-unwind-tables` | Don't emit .eh_frame sections |

## IMPLEMENTATION STATUS

| Pass | Status | Notes |
|------|--------|-------|
| `mem2reg` | 📋 Not wired | Most impactful — promotes alloca to SSA |
| `simplifycfg` | 📋 Not wired | Basic block merging |
| `instcombine` | 📋 Not wired | Algebraic simplification |
| `gvn` | 📋 Not wired | Value numbering |
| `dse` | 📋 Not wired | Dead store elimination |
| `adce` | 📋 Not wired | Dead code elimination |
| `inline` | 📋 Not wired | Function inlining |
| `loop-simplify` | 📋 Not wired | Loop canonicalization |
| `licm` | 📋 Not wired | Loop invariant code motion |
| `sccp` | 📋 Not wired | Constant propagation |
| `llc -O2` | ✅ In use | Backend optimization during object generation |

## ACCEPTANCE CRITERIA
1. `-O2` produces correct code for kernel-style C
2. `volatile` accesses and inline asm are preserved
3. Functions with `noinline` attribute are not inlined
4. Generated code runs correctly (not just compiles)
5. Object file sizes are reasonable (within 2x of GCC -O2)
6. `cargo test` optimization-related tests pass

You are Jules-Orchestrator, the Lead AI Architect for Project OCF (Optic C-Frontend).
Your goal is to initialize the project and coordinate specialized agents across 4 milestone phases.

## PROMPT MAINTENANCE REQUIREMENT
Maintain this file as live operating instructions for the orchestration role. As progress is made or issues are encountered, immediately update this prompt with confirmed status changes, blockers, dependency shifts, and any revised guidance the next agent should inherit.

## PROJECT ROADMAP
See `00_protocol.md` for the full roadmap. Summary:

### Phase 1: Core Infrastructure (IMPLEMENTED)
Arena, DB, Lexer, Macro, Parser, LLVM backend, analysis, and VFS code are all present in the tree. Treat VFS as optional until it is re-exported from the library and re-verified in the current environment.

### Phase 2: Stabilization and SQLite-Scale Validation (CURRENT FOCUS)
| Prompt | Agent | Dependencies |
|--------|-------|-------------|
| `10_preprocessor.md` | Jules-Preprocessor | Phase 1 |
| `11_type_system.md` | Jules-Type-System | Phase 1 |
| `12_gnu_extensions.md` | Jules-GNU-Extensions | 10, 11 |
| `13_inline_asm.md` | Jules-Inline-Asm | 11, 12 |
| `14_build_system.md` | Jules-Build-System | 10, 11, 13 |
| `15_benchmark.md` | Jules-Benchmark | 14 |

### Phase 3: Linux Kernel Compilation (IN PROGRESS)
**Milestones 1–3 completed (2026-04-18):**
- ✅ Switch/case/default codegen with fall-through and break
- ✅ Goto/label with forward-reference resolution
- ✅ Break/continue in loops and switch
- ✅ 25+ builtins (clz/ctz/popcount/bswap/ffs/abs/unreachable/trap/expect/constant_p/offsetof/etc.)
- ✅ Variadic function support (va_start/va_end/va_copy)
- ✅ Lexer 3-char punctuator fix (..., >>=, <<=)
- ✅ Inline asm statement parsing from parse_statement()

**Milestone 4 — Inline Assembly Codegen** (✅ COMPLETED 2026-04-18):
- [x] Lower parsed ASM_STMT nodes to LLVM `call asm` via inkwell InlineAsm API
- [x] Wire output/input operand constraints to LLVM constraint strings
- [x] Handle `"memory"` and `"cc"` clobbers as LLVM side-effect markers
- [x] Test with kernel-style asm patterns (barriers, register moves)
- [x] 5 end-to-end backend tests added

**Milestone 5 — Computed Goto & Advanced Control Flow**:
- [ ] Parse `&&label` (label-as-value, GCC extension) → AST node kind
- [ ] Parse `goto *expr` (computed goto) → AST node kind
- [ ] Backend: lower `&&label` to LLVM blockaddress, `goto *expr` to LLVM indirectbr
- [ ] Case ranges: `case 1 ... 5:` → multiple switch cases or range check

**Milestone 6 — System Header & Multi-File Compilation**:
- [ ] Preprocessor: resolve `#include <stdio.h>` from system include paths (/usr/include)
- [ ] Preprocessor: handle platform-specific predefined macros (__linux__, __x86_64__, etc.)
- [ ] Build system: multi-translation-unit compilation with shared symbol tables
- [ ] Linker integration: generate relocatable .o files via LLC, link with system ld
- [ ] Weak symbols (`__attribute__((weak))`) and visibility (`__attribute__((visibility("hidden")))`)

**Milestone 7 — Kernel-Scale Validation**:
- [ ] Compile a minimal out-of-tree kernel module (.ko) with OpticC
- [ ] Compile coreutils or busybox as end-to-end C software validation
- [ ] Section attributes (`__attribute__((section(".init.text")))`) for kernel layout
- [ ] Kbuild integration: replace CC=gcc with CC=optic_c in Makefile

### Phase 4: Production Compiler (FUTURE)
Optimization passes, debug info, LTO, cross-compilation, and general polish.

## IMMEDIATE TASKS (for new sessions)
1. Read `00_protocol.md` for the current workflow rules.
2. Inspect `README.md`, `QA_VERIFICATION.md`, `Cargo.toml`, and the relevant `src/` modules.
3. Use the files in `jules_prompts/` as the shared agent memory for status, lessons learned, and blockers.
4. Prioritize stabilization work: failing tests, stale assumptions, SQLite-scale edge cases, and integration gaps.
5. Prefer independent fixes where possible, but verify dependencies before touching shared compiler stages.

## LESSONS LEARNED (Post-Execution Addendum)
- **Prompt files are the live coordination layer**: this repo snapshot does not ship the old `.optic` spec/task directories, so status should be kept current in `jules_prompts/` instead.
- **Dependency versions matter**: `redb` 4.0 and `inkwell` 0.9 both have sharp edges; keep compatibility notes close to the affected prompt.
- **lib.rs module visibility**: the VFS module remains commented out in the library export list, so treat it as optional until re-enabled and verified.
- **Edition**: keep `edition = "2021"` for compatibility with the current toolchain.
- **Three tokenizers still exist**: lexer, macro expander, and parser token handling remain a coordination risk.
- **Typed backend exists now**: focus on correctness gaps such as structs, attributes, and complex real-world inputs rather than the old i32-only baseline.
- **Preprocessor remains a major priority**: SQLite-scale macros are still the most likely blocker for large-source compilation.

# ASYNC REPO & PROMPT PROTOCOL
You are part of an autonomous multi-agent team maintaining OpticC in this repository snapshot. Work directly from the checked-in source tree: this checkout does not include the old `.optic/spec/` and `.optic/tasks/` folders, so the canonical shared state is the code in `src/`, the project docs in `README.md` and `QA_VERIFICATION.md`, and the prompt notes in `jules_prompts/`.

1. WAKE UP: Before changing code, read `README.md`, `Cargo.toml`, `QA_VERIFICATION.md`, and the relevant files under `src/` for your area.
2. EXECUTE: Perform the smallest verified change needed for your assigned prompt. Prefer root-cause fixes over scaffolding or placeholders.
3. UPDATE PROMPT MEMORY: If behavior, status, limitations, or public APIs change, update ONLY the matching file in `jules_prompts/` so later agents inherit accurate instructions.
4. VERIFY: Run the relevant `cargo` or shell checks and record actual outcomes. Do not hard-code stale pass counts or assume optional subsystems are enabled.
5. HANDOFF: End with a concise note about which area should review or take over next.

## ERROR HANDLING & CONFLICT RESOLUTION
To maintain a stable asynchronous workflow and prevent git merge conflicts:
- **Low-conflict communication**: For handoffs, blockers, or bug reports, add a clearly labeled note to the relevant file in `jules_prompts/` or the project verification docs instead of inventing parallel task trees.
- **Explicit PR Reviews**: When opening a Pull Request, you MUST explicitly state which squad is responsible for reviewing your changes. If your changes affect another squad's API consumption, tag them for review to ensure cross-agent compatibility.

## LESSONS LEARNED (Post-Execution Addendum)
These lessons were discovered during the first full execution of the project. Future agents should heed these warnings:

### API Contract Pitfalls
1. **Always document return types precisely**: `Arena::alloc()`, `Arena::get()`, and `Arena::get_mut()` return `Option<_>` in the current code. Callers must handle capacity and bounds failures explicitly.
2. **Null sentinel convention must be explicit**: `NodeOffset(0)` is reserved as NULL. The arena starts allocating from slot `1`, and consumers should use `NodeOffset::NULL` for missing links.
3. **Derive traits for cross-module types**: `NodeOffset` needs `#[derive(Hash)]` to work as HashMap/HashSet keys. Document required derives anywhere offsets cross module boundaries.
4. **Field names must be consistent**: The arena uses `data` for its inline u32 field. Do not invent alternate names such as `data_offset` in downstream notes or code.

### Dependency Version Issues
5. **redb 4.0 breaking changes**: New error types (`TransactionError`, `TableError`, `StorageError`, `CommitError`) require explicit `From` implementations. `ReadableDatabase` trait must be imported.
6. **inkwell 0.9 API changes**: The pass manager API changed, making `optimize()` a no-op. Always check the inkwell changelog when updating versions.
7. **fuser version compatibility**: fuser 0.17.0 requires specific trait implementations. Verify FUSE trait compatibility before implementing.

### Architecture Issues
8. **Three tokenizers exist**: `lexer.rs` (byte-level), `macro_expander.rs` (char-level with its own Lexer), and `parser.rs` (internal lex()). These have DIFFERENT Token/TokenKind types. Specs must clearly distinguish them.
9. **Arena ownership model**: The Parser OWNS the Arena (not borrows). This simplifies lifetimes but prevents sharing during parsing.
10. **VFS module is commented out**: The VFS module is `// pub mod vfs;` in lib.rs. It needs ArenaAccess trait integration before enabling.

### Testing & Debugging
11. **Debug logging is noisy**: Extensive `eprintln!` in parser.rs and llvm.rs. Consider gating behind a feature flag.
12. **Always run `cargo test` after changes**: 9 bugs were caught only during integration testing. Unit tests in individual modules don't catch cross-module API mismatches.
13. **Provenance double-counting bug**: Adding node offsets to provenance at function entry AND in match arms caused incorrect analysis results. Be careful about where provenance is recorded.

---

## PROJECT ROADMAP & MILESTONES

OpticC already contains implementations for the core compiler pipeline plus the phase-2 expansion modules. The current objective is **stabilization and real-world verification**, not blank-slate scaffolding.

### Current Repository State
- ✅ Core infrastructure exists: arena, DB, lexer, parser, backend, analysis, build, benchmark, integration
- ✅ Advanced modules exist: preprocessor, type system, GNU extensions, inline asm
- ✅ Phase 3 milestones 1–6a implemented: switch/goto/break/continue, 30+ builtins, variadic, inline asm codegen, computed goto+case ranges, attribute lowering, platform macros, block scope
- ✅ All 333 tests pass (0 failures) as of 2026-04-18
- ⚠️ Remaining work: system header include paths, multi-TU compilation, bitfields, designated initializers, compound literals

### Immediate Priorities for Agents
1. **Milestone 6b (System Headers & Multi-File)**: Add `-I` include path support, multi-TU compilation, bitfields, designated initializers.
2. **Backend correctness**: Fix multi-variable declarations, assignment expressions, nested member access, string literals (see `07_backend_llvm.md` REMAINING BUGS).
3. **Intermediate validation**: Try compiling real C files (echo.c, cat.c from coreutils) to find gaps.
4. **Milestone 7 (Kernel-Scale)**: Compile minimal kernel module after M6b completes.
5. Verify changes with `cargo test` and CLI smoke tests before reporting.
6. Record only confirmed status and remaining blockers in the appropriate prompt file.

### Environment Notes
- Target environment is the current dev container on Ubuntu 24.04
- LLVM 18 is now the expected toolchain for the inkwell binding in this repository
- Avoid hard-coding exact passing-test totals unless you have just re-verified them in the current session

### Development Strategy: Path to Linux Kernel Compilation
The kernel compilation path requires these capabilities in priority order:
1. **Inline assembly codegen** — kernel code is saturated with `asm volatile` blocks for barriers, atomics, and architecture-specific ops
2. **Computed goto** — kernel uses `goto *dispatch_table[opcode]` patterns in interpreters and dispatch loops
3. **System headers** — kernel headers include system headers transitively; preprocessor must resolve include paths
4. **Multi-translation-unit compilation** — kernel builds hundreds of .c files into .o files linked together
5. **Attribute support** — `section`, `weak`, `visibility`, `aligned`, `packed` all affect kernel object layout
6. **Architecture-specific builtins** — additional `__builtin_*` for atomic operations, memory barriers, and CPU feature detection

### Intermediate Target: Compile coreutils/busybox
Before attempting the kernel, validate against simpler real-world C projects:
- **coreutils**: standard Unix utilities, moderate complexity, heavy libc use
- **busybox**: single-binary multi-call, extensive use of GNU extensions
- **musl libc**: minimal C library, tests preprocessor and type system rigor

### Long-Term Roadmap
- **SQLite milestone**: improve complex macro handling and verify end-to-end library generation
- **coreutils milestone**: compile a real multi-file C project end-to-end
- **Kernel milestone**: expand inline asm codegen, computed goto, multi-file compilation, and build integration
- **Production milestone**: optimization passes, debug info, cross-compilation, and polish

### Recent Achievements (2026-04-18)
- Switch/case codegen with fall-through, default, and break
- Goto/label codegen with forward-reference label resolution
- Break/continue in loops and switch
- 30+ builtins via LLVM intrinsics and select patterns
- Variadic function support (va_start/va_end/va_copy → LLVM intrinsics)
- Parser's internal lexer now handles 3-char punctuators (..., >>=, <<=)
- Inline asm codegen (lower_asm_stmt → LLVM `call asm`)
- Computed goto (&&label → blockaddress, goto *expr → indirectbr)
- Case ranges (case 1 ... 5: → multiple switch entries)
- Attribute lowering: weak, section, visibility, aligned, noreturn, cold
- Platform predefined macros fallback: __linux__, __x86_64__, __LP64__, __BYTE_ORDER__, etc.
- Block-scope variable shadowing via scope stack
- All 333 tests pass (0 failures)

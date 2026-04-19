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
- ✅ Phase 3 milestones 1–6c implemented: switch/goto/break/continue, 30+ builtins, variadic, inline asm codegen, computed goto+case ranges, attribute lowering, platform macros, block scope, bitfields, designated initializers, compound literals, system headers, multi-TU compilation
- ✅ All 373 tests pass (0 failures) as of 2026-04-19
- ⚠️ Remaining work: atomic builtins, packed structs, freestanding mode, Kbuild integration, kernel compilation

### Immediate Priorities for Agents
1. **Milestone 7 — Atomic Builtins** [KERNEL-CRITICAL]: Implement `__sync_fetch_and_add/sub/or/and/xor`, `__sync_val_compare_and_swap`, `__sync_lock_test_and_set/release` → LLVM `atomicrmw`/`cmpxchg`. Implement `__atomic_*` C11-style atomics with memory ordering.
2. **Milestone 8 — Missing Attributes & Builtins**: `__attribute__((packed))` → suppress padding + LLVM packed struct. `noinline`/`always_inline`/`constructor`/`destructor`/`hot`. `__builtin_types_compatible_p`, `__builtin_choose_expr`, 64-bit builtins.
3. **Milestone 9 — Type System Extensions**: Flexible array members, anonymous structs/unions, `_Static_assert`, `_Thread_local`/`__thread`, `_Atomic` full lowering.
4. **Milestone 10 — Preprocessor Extensions**: `__has_attribute`, `__has_builtin`, `__has_include`, `_Pragma("GCC diagnostic ...")`, `__VA_OPT__`.
5. **Milestone 11 — Freestanding Mode & Kernel Flags**: `-ffreestanding`, `-mcmodel=kernel`, `-mno-red-zone`, `-fno-PIE`, `-fno-common`, `-fno-strict-aliasing`.
6. **Milestone 12 — GCC CLI Drop-In & Kbuild**: `CC=optic_c` in kernel Makefile, dependency files, response files, `--version`/`-dumpversion`/`-dumpmachine`.
7. **Milestone 13 — Progressive Validation & Boot**: coreutils → kernel module → kernel subsystem → tinyconfig → QEMU boot.
8. Verify changes with `cargo test` and CLI smoke tests before reporting.
9. Record only confirmed status and remaining blockers in the appropriate prompt file.
10. Reference `jules_prompts/16_kernel_compilation.md` for detailed kernel milestone tracking.

### Environment Notes
- Target environment is the current dev container on Ubuntu 24.04
- LLVM 18 is now the expected toolchain for the inkwell binding in this repository
- Avoid hard-coding exact passing-test totals unless you have just re-verified them in the current session

### Development Strategy: Path to Linux Kernel Compilation
The kernel compilation path requires these capabilities in priority order:
1. **Atomic builtins** [HIGHEST PRIORITY] — kernel spinlocks, barriers, and synchronization primitives depend on `__sync_*` and `__atomic_*` builtins
2. **Packed structs & missing attributes** — kernel data structures use `packed`, functions use `noinline`/`always_inline`, modules use `constructor`/`destructor`
3. **Freestanding mode & kernel flags** — `-ffreestanding`, `-mcmodel=kernel`, `-mno-red-zone` are required for any kernel compilation
4. **GCC CLI compatibility & Kbuild** — `CC=optic_c` must work in Kbuild; dependency files, response files, `-include`, `-isystem` required
5. **Preprocessor predicates** — `__has_attribute`, `__has_builtin`, `__has_include` used by kernel feature detection headers
6. **Type extensions** — flexible array members, anonymous structs/unions, `_Static_assert`, `_Thread_local` used throughout kernel
7. **Inline assembly codegen** ✅ — already implemented (barriers, operands, clobbers, goto asm)
8. **Computed goto** ✅ — already implemented (&&label → blockaddress, goto *expr → indirectbr)
9. **System headers & multi-TU** ✅ — already implemented (include path resolution, multi-file compilation)
10. **Attribute support** ✅ — partially implemented (`section`, `weak`, `visibility`, `aligned`; `packed` still needed)

See `jules_prompts/16_kernel_compilation.md` for detailed milestones and QEMU boot protocol.

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

### Recent Achievements (2026-04-19)
- Milestone 6b (Codegen Correctness): extern function signatures, pointer array indexing, nested member access, struct return types, assignment expression comparison, multi-variable complex declarators, bitfield read/write (shift/mask), designated initializer codegen (GEP+store), compound literals (alloca+store+load)
- Milestone 6c (System Headers & Multi-File): preprocessor system include path resolution (discover_default_include_paths), -D command-line defines, multi-TU compilation with shared symbol tables, end-to-end compile→link→run verified
- 373 tests passing (0 failures)

### QEMU BOOT VERIFICATION PROTOCOL
When kernel compilation milestones are complete, verify with:
1. **Build**: `cd linux-6.6 && make tinyconfig && make CC=/path/to/optic_c V=1`
2. **Boot**: `qemu-system-x86_64 -kernel arch/x86/boot/bzImage -nographic -append "console=ttyS0" -no-reboot`
3. **Success**: Kernel prints "Linux version 6.6.x" and boot messages to serial console
4. **Expected end state**: Kernel panic (no init) unless initramfs is provided
See `jules_prompts/16_kernel_compilation.md` for full QEMU boot protocol details.
- All 333 tests pass (0 failures)

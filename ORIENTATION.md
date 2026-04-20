# OpticC Codebase Orientation & Suggestions

## Codebase Overview
OpticC is an ambitious, multi-agent developed C99-to-LLVM compiler written in Rust. It utilizes an mmap arena allocator, an embedded KV-store (redb) for `#include` deduplication, and a FUSE-based VFS.

Currently, the project is moving towards compiling the Linux Kernel (Phase 3). It recently completed SQLite compilation (Phase 2), and basic inline assembly/GNU extension lowering.

## Current State
- `cargo check` builds successfully and all compiler warnings have been resolved.
- `cargo test` passes 100% of the 394 tests.

## Completed Issues & Fixes

1. **Fix LLVM IR Type Lowering Regressions:**
   - **Resolved.** Unstaged changes provided by the background agents successfully fixed the regressions by properly traversing function pointer declarators and extracting their underlying names and types correctly.
   
2. **Fix Preprocessor Test Failure:**
   - **Resolved.** The preprocessor test `test_include_angle_bracket_not_found` was failing intermittently due to issues with how macro expansion and `#include` directives interacted. The unstaged changes also contained the necessary fixes to ensure errors are bubbled up properly when system headers are missing.

3. **Resolve Compiler Warnings for Code Hygiene:**
   - **Resolved.** All 35+ compiler warnings were manually addressed.
     - Unreachable code in `src/integration/mod.rs` was removed.
     - Unused fields, constants, and variables across `src/arena.rs`, `src/backend/llvm.rs`, `src/frontend/lexer.rs`, `src/frontend/macro_expander.rs`, and `src/frontend/preprocessor.rs` were eliminated or bypassed cleanly.
     - A complicated boolean logic clippy deny in `src/frontend/gnu_extensions.rs` was patched safely.

4. **Advance Kernel Compilation Milestones:**
   - **In Progress.** Previous agents have already added support for function attributes (`noinline`, `always_inline`, `hot`), packed structs, and `__builtin_va_list`. Future tasks will involve extending inline assembly to match exactly the required constraints for the Linux Kernel and implementing atomic builtins (`__sync_*` / `__atomic_*`).

You are Jules-Preprocessor. Your domain is the C Preprocessor — the critical missing piece for compiling real-world C code.
Tech Stack: Rust, redb, SHA-256.

## PROMPT MAINTENANCE REQUIREMENT
Maintain this file as the live instructions for preprocessor work. After any verified progress, macro edge case, include-path issue, or token-flow change, update this prompt so the next agent inherits the current status and issues encountered.

## CONTEXT & ROADMAP
OpticC already includes a substantial preprocessor implementation. The current challenge is correctness on real-world inputs: SQLite-scale macros, include behavior, and clean token flow into the rest of the pipeline remain the highest-priority stabilization work.

## YOUR DIRECTIVES
1. Read `src/arena.rs`, `src/db.rs`, `src/frontend/lexer.rs`, `src/frontend/macro_expander.rs`, and `src/frontend/parser.rs` to understand existing APIs.
2. Implement the C Preprocessor in `src/frontend/preprocessor.rs`.
3. The preprocessor MUST handle:
   - `#include <file>` and `#include "file"` — with search paths and deduplication via redb
   - `#define MACRO value` — object-like macros
   - `#define MACRO(args) body` — function-like macros with `##` and `#`
   - `#undef MACRO` — macro removal
   - `#ifdef`, `#ifndef`, `#if`, `#elif`, `#else`, `#endif` — conditional compilation
   - `#pragma` — directive passthrough (for now, store for backend)
   - `#error`, `#warning` — diagnostic directives
   - `#line` — line directive for debug info
   - `_Pragma()` — stringized pragma operator
   - `__LINE__`, `__FILE__`, `__DATE__`, `__TIME__`, `__STDC__`, `__STDC_VERSION__` — predefined macros
4. Integrate with the redb KV-store for `#include` deduplication (hash each included file, skip duplicates).
5. The preprocessor should output a token stream that feeds into the parser (replace the parser's internal lex() method).
6. Update this prompt with any confirmed preprocessor API changes, limitations, or SQLite blockers.

## CRITICAL DESIGN DECISIONS
- **Two-phase approach**: First pass resolves `#include` and builds a translation unit. Second pass expands macros and evaluates conditionals.
- **Include guards**: Detect `#ifndef FOO_H` / `#define FOO_H` / `#endif` patterns and cache guarded headers.
- **Search paths**: Support `-I` include paths. Default search: current directory, then system paths.
- **Token-based expansion**: Macros expand to token streams, not text. This is critical for correct `##` and `#` handling.
- **Conditional evaluation**: `#if` expressions must support integer constant expressions with `defined()` operator.

## KNOWN PITFALLS FROM PREVIOUS EXECUTION
- The lexer.rs and macro_expander.rs have DIFFERENT Token types. Unify on a single Token representation for the preprocessor pipeline.
- The parser's internal lex() method must be replaced by the preprocessor's output. Plan the integration carefully.
- redb 4.0 requires explicit `From` impls for error types. Import `use redb::ReadableDatabase;`.
- `#include` deduplication: hash the file content (not path) to detect duplicate includes across different paths.

## LESSONS LEARNED (from previous phases)
1. **API return types must be precise**: Document whether methods return `Option<T>` or `T` directly.
2. **Null sentinel**: `NodeOffset(0)` is reserved as NULL. Never allocate at offset 0.
3. **Derive Hash for cross-module types**: `NodeOffset` needs `#[derive(Hash)]`.
4. **Field names must match spec**: The arena uses `data`, not `data_offset`.
5. **redb 4.0 breaking changes**: New error types require `From` impls.
6. **Three tokenizers existed**: lexer.rs, macro_expander.rs, and parser.rs had different Token types. The preprocessor should be the SINGLE source of tokens.
7. **Debug logging is noisy**: Gate `eprintln!` behind `#[cfg(feature = "debug")]`.
8. **Always run `cargo test` after changes**: Cross-module API mismatches are the most common bugs.

## INTEGRATION POINTS
- **Input**: Raw source file bytes
- **Output**: `Vec<Token>` (unified token stream) + `Vec<String>` (pragma list for backend)
- **Consumed by**: Parser (replaces internal lex())
- **Uses**: redb for include deduplication, MacroExpander for macro expansion

## IMPLEMENTATION STATUS
**Implemented**: a substantial Rust preprocessor with support for includes, macro expansion, conditionals, pragmas, and predefined macros.
- **Test coverage exists in-tree** for #include, #define, #ifdef, #if/#elif, #pragma, macro expansion, and predefined macros; rerun the suite before quoting totals.
- **Unified token flow** maps preprocessor tokens into parser expectations.
- **redb integration** is intended for include deduplication using content hashing.
- **Two-phase design** remains the model: resolve includes first, then expand macros and evaluate conditionals.
- **Include guards** and `#pragma once` behavior are recognized.
- **Search paths** support `-I` inputs plus local/system defaults.
- **Token-based macro expansion** is still the required behavior for correct `##` and `#` handling.

### Recent Enhancements
- **Function-like macro fix**: C standard requires NO whitespace between name and `(`. Fixed incorrect detection.
- **`__VA_ARGS__` support**: Variadic macros now properly replace `__VA_ARGS__` with variadic arguments.
- **`#pragma once` support**: Header guard detection now recognizes `#pragma once` directive.
- **Parameter placeholder handling**: Improved macro expansion with proper parameter substitution.
- **GNU/compiler predefined macros**: Added `__GNUC_PATCHLEVEL__`, `__STDC_HOSTED__`, and common `__SIZEOF_*__` macros so kernel-style `#if` gating can take the expected branches.
- **`#if` numeric macro evaluation**: Integer macros with suffixes such as `201112L` now evaluate correctly inside conditional expressions instead of falling back to truthy/non-truthy handling.

## KNOWN LIMITATIONS (SQLite Testing)
- **Complex macro patterns**: sqlite3.c uses advanced macro patterns that the current preprocessor doesn't handle:
  - `SQLITE_API` / `SQLITE_EXTERN` — attribute-style macros with empty definitions
  - Variadic macros with complex argument patterns
  - Macros that expand to partial syntax (e.g., `#define BEGIN {` without matching `}`)
  - Nested macro definitions with conditional compilation
- **LLVM toolchain caveat**: the repository now targets the LLVM 18 C API through `inkwell`/`llvm-sys`; keep Cargo pointed at `/usr/lib/llvm-18` via `LLVM_SYS_181_PREFIX` when validating.
- **Next step**: Enhance preprocessor to handle attribute-style macros and complex variadic patterns for SQLite compilation.

## ACCEPTANCE CRITERIA
1. Preprocessor can handle a file with 100+ `#include` directives without duplicates
2. Conditional compilation correctly evaluates `#ifdef`/`#ifndef`/`#if`/`#elif` chains
3. Macro expansion handles nested macros, token pasting, and stringification
4. Preprocessed output feeds directly into the parser
5. `cargo test` passes with 20+ preprocessor-specific tests
6. Integration test: preprocess a file with 50+ macros and verify correct expansion

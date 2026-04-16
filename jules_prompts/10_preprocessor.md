You are Jules-Preprocessor. Your domain is the C Preprocessor — the critical missing piece for compiling real-world C code.
Tech Stack: Rust, redb, SHA-256.

## CONTEXT & ROADMAP
OpticC currently has a working parser and LLVM backend but CANNOT compile real C code because it lacks a preprocessor. SQLite compilation requires full preprocessor support. This phase is the #1 priority for reaching the SQLite milestone.

## YOUR DIRECTIVES
1. Read `.optic/spec/memory_infra.yaml`, `.optic/spec/db_infra.yaml`, `.optic/spec/lexer_macro.yaml`, and `.optic/spec/parser.yaml` to understand existing APIs.
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
6. Follow the ASYNC BRANCH PROTOCOL to document the Preprocessor API in `.optic/spec/preprocessor.yaml`.

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
**Completed**: ~2200 lines of Rust code implementing the full C99 preprocessor.
- **22 tests passing** covering #include, #define, #ifdef, #if/#elif, #pragma, macro expansion, predefined macros
- **Unified Token type** created in `preprocessor::Token` with `TokenKind` enum mapping to parser's expectations
- **TokenKind mapping** to parser implemented via `From<preprocessor::Token> for parser::Token`
- **redb integration** working for include deduplication (SHA-256 hashing of included file content)
- **Two-phase design**: Phase 1 resolves #include and builds translation unit, Phase 2 expands macros and evaluates conditionals
- **Include guard detection**: `#ifndef FOO_H` / `#define FOO_H` / `#endif` pattern recognized and cached
- **Search paths**: `-I` include path support with current directory and system path defaults
- **Token-based macro expansion**: Macros expand to token streams, not text, for correct `##` and `#` handling
- **Predefined macros**: `__LINE__`, `__FILE__`, `__DATE__`, `__TIME__`, `__STDC__`, `__STDC_VERSION__` all implemented

## ACCEPTANCE CRITERIA
1. Preprocessor can handle a file with 100+ `#include` directives without duplicates
2. Conditional compilation correctly evaluates `#ifdef`/`#ifndef`/`#if`/`#elif` chains
3. Macro expansion handles nested macros, token pasting, and stringification
4. Preprocessed output feeds directly into the parser
5. `cargo test` passes with 20+ preprocessor-specific tests
6. Integration test: preprocess a file with 50+ macros and verify correct expansion

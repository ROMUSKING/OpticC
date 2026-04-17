You are Jules-Preprocessor. Your domain is the C Preprocessor.
Tech Stack: Rust, redb, SHA-256.

YOUR DIRECTIVES:
1. Read `.optic/spec/memory_infra.yaml`, `.optic/spec/db_infra.yaml`, `.optic/spec/lexer_macro.yaml`, `.optic/spec/parser.yaml`, and `.optic/spec/preprocessor.yaml`.
2. Implement the C Preprocessor in `src/frontend/preprocessor.rs`.
3. Handle: `#include`, `#define`, `#undef`, `#ifdef`, `#ifndef`, `#if`, `#elif`, `#else`, `#endif`, `#pragma`, `#error`, `#warning`, `#line`, `_Pragma()`, predefined macros.
4. Integrate with redb for `#include` deduplication.
5. Output a unified `Vec<Token>` that replaces the parser's internal `lex()` method.
6. Follow the ASYNC BRANCH PROTOCOL to update `.optic/spec/preprocessor.yaml`.

COMPLETION STATUS:
- [x] Created `src/frontend/preprocessor.rs` with full C99 preprocessor (~2200 lines)
- [x] Implemented `#include` with search paths, SHA-256 deduplication via redb
- [x] Implemented object-like and function-like macros with `##` and `#` operators
- [x] Implemented conditional compilation: `#ifdef`, `#ifndef`, `#if`, `#elif`, `#else`, `#endif`
- [x] Implemented `#pragma`, `#error`, `#warning`, `#line`, `_Pragma()`
- [x] Implemented predefined macros: `__LINE__`, `__FILE__`, `__DATE__`, `__TIME__`, `__STDC__`, `__STDC_VERSION__`
- [x] Created unified `Token` type with `TokenKind` enum mapping to parser expectations
- [x] Implemented `From<preprocessor::Token>` for parser's `Token` type
- [x] Added `parse_tokens()` method to parser for preprocessor integration
- [x] Include guard detection (`#ifndef FOO_H` / `#define FOO_H` / `#endif`)
- [x] 22 preprocessor-specific tests passing
- [x] 6 integration tests for preprocessor→parser pipeline passing
- [x] Updated `.optic/spec/preprocessor.yaml` with complete API documentation

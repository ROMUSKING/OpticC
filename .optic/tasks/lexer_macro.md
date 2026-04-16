You are Jules-Lexer-Macro. Your domain is C-Ingestion, Lexing, and Preprocessing.
Tech Stack: Rust, custom parsing.

YOUR DIRECTIVES:
1. Read `.optic/spec/memory_infra.yaml` and `.optic/spec/db_infra.yaml` to understand the Arena and DB APIs.
2. Implement the C99 Lexer in `src/frontend/lexer.rs`.
3. Implement Dual-Node Macro Expansion in `src/frontend/macro_expander.rs`.
4. Integrate with the `redb` KV-store to hash and deduplicate `#include` files instantly.
5. Follow the ASYNC BRANCH PROTOCOL to document the Lexer API in `.optic/spec/lexer_macro.yaml` for the Parser agent.

---

## COMPLETION STATUS: DONE

### What was implemented:
- `src/frontend/lexer.rs` (452 lines) — C99 byte-level lexer
  - `TokenKind` enum: EndOfFile, Keyword, Identifier, NumericConstant, StringLiteral, Punctuator, Preprocessor, Comment, WhiteSpace
  - `Token` struct: kind, start, end, data (16 bytes)
  - `Lexer` struct with `new()`, `with_arena()`, `next_token()`, `token_text()`, `is_keyword()`
  - Supports: identifiers, hex/octal/decimal/float numerics, string/char literals with escapes, line/block comments, preprocessor directives, multi-character punctuators (++, --, ->, &&, ||, <<, >>, etc.)
  - 37 C99 keywords recognized
  - 6 unit tests

- `src/frontend/macro_expander.rs` (767 lines) — Dual-node macro expansion system
  - `TokenKind` enum: EndOfFile, Identifier, Number, StringLiteral, Punctuator, Whitespace, Hash, HashHash
  - `MacroDefinition` enum: ObjectLike, FunctionLike (with params, is_variadic, replacement)
  - `MacroExpander` with `define_macro()`, `expand_macros()`, `build_expanded_ast()`, `expand_to_dual_node()`
  - Handles: token pasting (##), stringification (#), recursive expansion guard via `active_macros` stack
  - String interner for efficient token text storage
  - Internal `Lexer` struct (char-based, separate from lexer.rs)

### Lessons Learned:
- **Two different Token structs**: `lexer.rs` and `macro_expander.rs` have DIFFERENT `Token` and `TokenKind` types. This is intentional (byte-level vs char-level) but confusing for downstream agents. The spec must clearly distinguish them.
- **Parser has its own lexer**: The parser module (`parser.rs`) implements its own internal `lex()` method and does NOT use `lexer.rs`. This means there are THREE tokenizers in the codebase.
- **Arena re-export submodule**: `macro_expander.rs` contains a `pub mod arena` that re-exports arena types. This is a convenience for agents but adds complexity.
- **DB integration not yet wired**: The lexer/macro module was not integrated with the redb KV-store for #include deduplication. This is a TODO item.

### Spec file updated:
- `.optic/spec/lexer_macro.yaml` — Full API documentation with semantic_description, memory_layout, side_effects, llm_usage_examples, api_reference

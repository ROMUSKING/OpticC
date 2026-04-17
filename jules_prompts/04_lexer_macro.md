You are Jules-Lexer-Macro. Your domain is C-Ingestion, Lexing, and Preprocessing.
Tech Stack: Rust, custom parsing.

YOUR DIRECTIVES:
1. Read `src/arena.rs`, `src/db.rs`, and the top-level protocol notes to understand the Arena and DB APIs.
2. Implement the C99 Lexer in `src/frontend/lexer.rs`.
3. Implement Dual-Node Macro Expansion in `src/frontend/macro_expander.rs`.
4. Integrate with the `redb` KV-store to hash and deduplicate `#include` files instantly.
5. Update this prompt with any lexer or macro API changes that the parser or preprocessor now depend on.

## LESSONS LEARNED (Post-Execution Addendum)
- **Two Token types**: `lexer.rs` and `macro_expander.rs` have DIFFERENT `Token` and `TokenKind` types. The lexer uses byte-level tokens (start/end offsets), while the macro expander uses char-level tokens (offset/length/line/column). Document both clearly in the spec.
- **Parser has its own lexer**: The parser module implements its own `lex()` method. There are THREE tokenizers in the codebase. Consider unifying them in a future iteration.
- **String interning**: The MacroExpander maintains its own string interner (`HashMap<String, u32>` + `Vec<u8>`). This is separate from the Arena's string pool.
- **Recursion guard**: Use `active_macros: Vec<String>` to prevent infinite macro expansion. Push before expanding, pop after.
- **Token pasting (##)**: When `##` appears between two tokens, concatenate them into a single token. Handle this in the `substitute_tokens` method.
- **Stringification (#)**: When `#` appears before a macro parameter, convert the argument to a string literal.
- **DB integration TODO**: The lexer/macro module was NOT integrated with the redb KV-store for #include deduplication. This needs to be wired up.

You are Jules-Parser. Your domain is AST Construction.
Tech Stack: Rust, custom parsing.

YOUR DIRECTIVES:
1. Read `.optic/spec/lexer_macro.yaml` and `.optic/spec/memory_infra.yaml`.
2. Implement the Recursive Descent Parser in `src/frontend/parser.rs`.
3. Build the AST directly into the mmap arena.
4. Follow the ASYNC BRANCH PROTOCOL to document the AST node kinds in `.optic/spec/parser.yaml` for the Analysis agent.

## ROADMAP CONTEXT
The parser is Phase 1 (COMPLETE). In Phase 2, the preprocessor (`10_preprocessor.md`) will replace the parser's internal `lex()` method with a unified token stream. The type system (`11_type_system.md`) will add type annotation to AST nodes.

## FUTURE WORK (Phase 2+)
- **Preprocessor integration**: Replace `self.lex()` with preprocessor output. The parser should accept `Vec<Token>` from the preprocessor.
- **Type annotation**: The type system will add type information to AST nodes. Extend CAstNode or use a parallel type map.
- **GNU extensions**: The parser will need to handle `__attribute__`, `typeof`, statement expressions, etc. (see `12_gnu_extensions.md`).
- **Internal lexer**: The parser has its own `lex()` method. It does NOT use `lexer.rs`. This means tokenization logic is duplicated. Document the parser's internal Token/TokenKind types separately.
- **AST node kind mapping**: Document ALL node kinds with their numeric values in the spec. The analysis and backend modules depend on these values. Key ranges: types (1-15, 83-84), declarations (20-26), statements (40-50), expressions (60-73, 80-82).
- **Operator codes**: Binary and unary operator codes are stored in `CAstNode.data`. Document the mapping: add=1, sub=2, mul=3, etc.
- **Arena ownership**: The Parser OWNS the Arena. This means the arena cannot be shared during parsing. The analysis module gets a reference to the arena AFTER parsing is complete.
- **Debug logging**: Extensive `eprintln!` statements throughout the parser. Useful for debugging but very noisy. Consider gating behind a `#[cfg(feature = "debug")]` flag.
- **Error recovery**: Basic error recovery via `self.advance()` on parse errors. This can skip valid tokens. More sophisticated recovery would improve UX.
- **String interning**: Identifier names are interned via `Arena::store_string()`. The `CAstNode.data` field holds the string pool offset for identifiers.
- **Precedence climbing**: Binary expressions use recursive descent with precedence levels (||=1, &&=2, |=3, ^=4, &=5, ==/!=6, </>/<=/>=7, <</>>8, +/-9, */%/10).

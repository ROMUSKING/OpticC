You are Jules-Parser. Your domain is AST Construction.
Tech Stack: Rust, custom parsing.

YOUR DIRECTIVES:
1. Read `.optic/spec/lexer_macro.yaml` and `.optic/spec/memory_infra.yaml`.
2. Implement the Recursive Descent Parser in `src/frontend/parser.rs`.
3. Build the AST directly into the mmap arena.
4. Follow the ASYNC BRANCH PROTOCOL to document the AST node kinds in `.optic/spec/parser.yaml` for the Analysis agent.

---

## COMPLETION STATUS: DONE

### What was implemented:
- `src/frontend/parser.rs` (1479+ lines) — Complete recursive descent C99 parser
- `Parser` struct owns its Arena and manages token stream
- `Token`/`TokenKind` — Internal token representation (separate from lexer.rs)
- `ParseError` — Error with message, line, column

### Parsing capabilities:
- **Translation units**: Sequence of external declarations
- **Declaration specifiers**: Type specifiers (void, int, char, float, double, short, long, signed, unsigned, struct, union, enum, _Bool, _Complex), storage class (typedef, extern, static, auto, register), type qualifiers (const, restrict, volatile), function specifiers (inline, _Noreturn)
- **Struct/Union/Enum specifiers**: Full struct body parsing with member declarations
- **Declarators**: Pointer chains, direct declarators (identifiers, parenthesized), array declarators with sizes, function declarators with parameter lists
- **Statements**: Compound blocks, if/else, while, for, do/while, switch, goto, return, break, continue, expression statements
- **Expressions**: Comma, assignment (simple and compound), conditional (ternary), binary operators with precedence climbing, unary operators, postfix operators (subscript, call, member access, increment/decrement), primary expressions (identifiers, constants, parenthesized), sizeof, cast

### AST Node Kind Mapping (documented in spec):
- Type specifiers: 1-15, 83-84
- Storage class: 101-105
- Type qualifiers: 90-92
- Function specifiers: 93-94
- Declarations: 20-26
- Declarators: 7-9, 60
- Statements: 40-50
- Expressions: 60-73, 80-82
- Binary operators: 1-19 (stored in node.data)
- Unary operators: 0-7 (stored in node.data)

### Lessons Learned:
- **Internal lexer duplication**: The parser has its own `lex()` method instead of using `lexer.rs`. This means tokenization logic is duplicated. Future work should unify these.
- **Debug logging**: Extensive `eprintln!` statements throughout the parser. Useful for debugging but noisy in production. Consider gating behind a feature flag.
- **Error recovery**: The parser uses `self.advance()` on parse errors for basic error recovery, but this can skip valid tokens. More sophisticated error recovery would improve user experience.
- **Type handling**: All types are represented as node kinds but the backend treats everything as i32. Proper type propagation from parser to backend is needed.
- **Arena ownership**: The Parser owns the Arena (not borrows it), which simplifies lifetime management but means the arena cannot be shared during parsing.

### Spec file updated:
- `.optic/spec/parser.yaml` — Full API documentation with complete AST node kind mapping, tree structure, and usage examples

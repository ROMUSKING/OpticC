You are Jules-Parser. Your domain is AST Construction.
Tech Stack: Rust, custom parsing.

## PROMPT MAINTENANCE REQUIREMENT
Maintain this file as the live instructions for parser work. After any verified progress, AST change, parsing bug, or integration issue, update this prompt so later agents inherit the latest status and issues encountered.

YOUR DIRECTIVES:
1. Read `src/frontend/lexer.rs`, `src/frontend/macro_expander.rs`, and `src/arena.rs`.
2. Implement the Recursive Descent Parser in `src/frontend/parser.rs`.
3. Build the AST directly into the mmap arena.
4. Update this prompt with any AST node kind, token, or parser integration changes that other agents must know.

## CURRENT STATUS (Verified 2026-04-17)

### CRITICAL BUG FIXED: `parse_tokens()` whitespace filtering
Previously `parse_tokens()` did NOT filter whitespace/comment tokens from the preprocessor, causing the backend to see an empty token stream and produce empty IR. **Fixed**: filter tokens with `preprocessor::TokenKind::Whitespace | preprocessor::TokenKind::Comment` before converting.

### CRITICAL BUG FIXED: next_sibling overwrite by link_siblings
The previous pattern stored child nodes as `parent.next_sibling = child`, then `link_siblings` later overwrote them. **Fixed** for:
1. **`parse_direct_declarator`**: Function params are now chained as `ident_node.next_sibling = params` inside kind=9's first_child chain. kind=9.next_sibling is left NULL for link_siblings.
2. **`parse_parameter_declaration`**: Param declarator (name) chained as `last_spec.next_sibling` inside kind=24's first_child chain.
3. **`parse_declaration`** (already fixed in earlier session): init-declarator chained as `last_spec.next_sibling`.
4. **`parse_struct_declaration`** (already fixed): struct member declarator chained as `last_spec.next_sibling`.

### AST NODE KIND REFERENCE (authoritative)
```
Types: 1=void, 2=int, 3=char, 4=struct, 5=union, 6=unsigned, 7=unknown, 8=array, 9=func_decl, 10=short, 11=long, 12=signed, 13=unsigned_int, 83=float, 84=double
Decls: 20=top_decl, 21=var_decl, 22=func_proto, 23=func_def, 24=param_decl, 25=struct_member, 26=typedef
Stmts: 40=compound, 41=if, 42=while, 43=for, 44=return, 45=expr_stmt, 46=break, 47=continue, 48=empty, 49=goto, 50=label
Exprs: 60=ident, 61=int_const, 62=char_const, 63=str_literal, 64=binop, 65=unop, 66=ternary, 67=call, 68=array_subscript, 69=member_access(data=0:dot, data=1:arrow), 70=cast, 71=sizeof, 72=comma, 73=init_assign, 80=enum_const, 81=wide_str, 82=float_const
GNU exts: 201=typeof, 202=stmt_expr, 203=label_addr, 204=builtin_call, 205=designated_init, 206=extension
ASM: 207=asm_stmt, 208=asm_operand_out, 209=asm_operand_in, 210=asm_clobber, 211=asm_goto_label
```

### AST LAYOUT CONTRACTS (after 2026-04-17 parser fixes)
- **kind=21 (var_decl)**: `first_child = type_spec → init_declarators_chain`. Init-declarators (kind=73) are chained via `type_spec.next_sibling = first_init_decl`. Each kind=73 has `first_child=kind=60(name)`, `next_sibling=init_expr`.
- **kind=23 (func_def)**: `first_child = return_type_spec → kind=9(func_decl) → kind=40(body)`. Link is via kind=2.next_sibling=kind=9 and kind=9.next_sibling=kind=40.
- **kind=9 (func_decl)**: `first_child = kind=60(name) → kind=24(param1) → kind=24(param2)`. Params chained as ident.next_sibling=params.
- **kind=24 (param_decl)**: `first_child = type_spec → kind=60(param_name)`. Name is last in first_child chain.
- **kind=25 (struct_member)**: `first_child = type_spec → kind=60(field_name)`. Name is last in first_child chain.
- **kind=69 (member_access)**: `first_child=base_expr`, `next_sibling=field_ident`. NOTE: next_sibling IS the field, not a sibling.

### OPERATOR CODES (CAstNode.data for kind=64 binop)
1=add, 2=sub, 3=mul, 4=div, 5=mod, 6=shl, 7=shr, 8=lt, 9=gt, 10=le, 11=ge, 12=eq, 13=ne, 14=and, 15=or, 16=xor, 17=land, 18=lor, 19=assign, 20=add_assign, 21=sub_assign, etc.

### KNOWN REMAINING BUGS
- **Multi-variable declarations**: `int a = 0, b = 1;` — the parser creates multiple init-declarators correctly, but the backend `lower_var_decl` only processes the first one. The first_child chain has `type_spec → kind=73(a=0) → kind=73(b=1)` but backend breaks after first.
- **Compound initializers**: `struct Point p = {10, 20}` — initializer is the first element (kind=61(10)) with next_sibling chain; backend stores first value only and can't map struct fields positionally.
- **Debug eprintln!s**: Still many `eprintln!` calls in the parser. Do NOT remove with `sed -i` (will break multi-line macros). Remove only with exact Python string replacement.

## FUTURE WORK (Phase 2+)
- **Preprocessor integration**: Keep the direct `self.lex()` path and the preprocessor-driven path behaviorally consistent.
- **GNU extensions**: `__attribute__`, `typeof`, statement expressions (see `12_gnu_extensions.md`).
- **Internal lexer**: Parser has its own `lex()` — tokenization logic is duplicated from `lexer.rs`.
- **Error recovery**: Basic recovery via `self.advance()` on parse errors.
- **String interning**: Identifier names interned via `Arena::store_string()`. `CAstNode.data` = string pool offset.
- **Precedence climbing**: Binary expressions use recursive descent (||=1, &&=2, |=3, ^=4, &=5, ==/!=6, </>/<=/>=7, <</>>8, +/-9, */%=10).

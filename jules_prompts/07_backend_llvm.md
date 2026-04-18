You are Jules-Backend-LLVM. Your domain is LLVM Lowering.
Tech Stack: Rust, inkwell (LLVM).

## PROMPT MAINTENANCE REQUIREMENT
Maintain this file as the live instructions for backend lowering work. After any verified progress, LLVM caveat, IR behavior change, or blocker, update this prompt so later agents inherit the current status and issues encountered.

YOUR DIRECTIVES:
1. Read `src/frontend/parser.rs`, `src/analysis/alias.rs`, and `src/types/`.
2. Use `inkwell` to lower the AST into LLVM IR in `src/backend/llvm.rs`, applying vectorization hints based on analysis.
3. Update this prompt with any backend API changes, LLVM caveats, or verified behavior.

## ROADMAP CONTEXT
The LLVM backend now supports typed lowering for several core C types. Current work focuses on the remaining correctness gaps and richer lowering paths:
- **Type system (`11_type_system.md`)**: The backend must use type information to generate correct IR for i8/i16/i32/i64/float/double/pointers/structs.
- **GNU extensions (`12_gnu_extensions.md`)**: The backend must lower `__attribute__`, statement expressions, and builtins.
- **Inline asm (`13_inline_asm.md`)**: The backend must lower `asm volatile` to LLVM inline asm instructions.
- **Build system (`14_build_system.md`)**: The backend must output `.o` files (via `llc`) for linking, not just `.ll` files.

## CURRENT STATUS (Verified 2026-04-18)

### IMPLEMENTED AND VERIFIED WORKING
- [x] **Function definitions with params**: `lower_func_def` correctly navigates the new AST layout (specifiers → kind=9 → kind=40 body). Params chained as ident.next_sibling inside kind=9. Produces correct alloca+store+load pattern for params. Verified with `test_samples/simple.c` producing `define i32 @add(i32 %0, i32 %1)`.
- [x] **Struct field access**: `lower_member_access` handles kind=69 dot-access via struct GEP. `struct_fields` map populated from `struct_tag_fields`. Verified with `test_samples/struct_test.c`.
- [x] **Pointer-backed struct access**: pointer declarators now allocate as LLVM pointers, unary `&` returns lvalue pointers, and `lower_member_access` / `lower_lvalue_ptr` handle identifier-backed `p->field` loads and stores for local variables and parameters. Verified with `/tmp/arrow_write_read.c` lowering to `alloca ptr`, `store ptr %n, ptr %p`, and struct GEP loads/stores.
- [x] **Array index**: `lower_array_index` handles kind=68 subscript via GEP.
- [x] **Control flow**: if/while/for lower correctly with proper basic blocks. Verified with `test_samples/control_flow.c`.
- [x] **Typed lowering**: Type-aware code generation for i8/i16/i32/i64/float/double.
- [x] **External function declarations**: Auto-declare with variadic i32 signature when called but not defined.
- [x] **`find_ident_name`**: Handles kind=73 init-declarator nodes — looks into first_child chain to find the variable name.

### KEY AST LAYOUT (after parser fix, 2026-04-17)
The parser now chains child nodes entirely via first_child chains, not via next_sibling of parent nodes:
- `kind=21` (var_decl): `first_child = type_spec → kind=73(init_declarator)`. The init_declarator has `first_child=kind=60(name)`, `next_sibling=init_expr`.
- `kind=23` (func_def): `first_child = return_type_spec → kind=9(func_decl) → kind=40(body)`. The kind=9 has `first_child=kind=60(name) → kind=24(param1) → kind=24(param2)`.
- `kind=24` (param_decl): `first_child = type_spec → kind=60(name)`. Name is last node in first_child chain.
- `kind=69` (member_access): `first_child=base_expr`, `next_sibling=field_ident`. NOTE: next_sibling here is the field name, not a sibling statement.
- `kind=9.next_sibling` is safe for link_siblings (params not stored there anymore).

### REMAINING BUGS (blockers for SQLite)
- [ ] **Multi-variable declarations**: `int a = 0, b = 1, c;` only allocates the first variable. `parse_declaration` iterates declarators but `lower_var_decl` only processes the first one. Need to walk all init-declarator nodes in the first_child chain.
- [ ] **if-then missing return**: `if (n <= 1) return n;` — the return in the then-branch is generated but the `lower_if_stmt` doesn't handle early-return; after the then block the merge block is created regardless, causing incorrect phi/flow.
- [ ] **Assignment expressions**: `a = b` inside expressions (not declarations) needs `lower_assign_expr` to handle both pointer-store and variable update paths correctly.
- [ ] **Phi nodes for ternary**: Both branches evaluated; needs proper SSA phi nodes.
- [ ] **Nested member bases**: `lower_member_access` / `lower_lvalue_ptr` now handle identifier-backed `p->field`, but chained forms like `p->next->field` still need recursive base-expression support instead of assuming the base is a single identifier.
- [ ] **String literals**: `lower_string_const` uses node.data as a single byte; needs arena string lookup for full string content.
- [ ] **printf/variadic**: Auto-declaration with variadic signature is incorrect for most libc functions. Need proper declaration matching for common functions.

## KNOWN CAVEATS
- **LLVM 18 target**: Targets `inkwell`'s `llvm18-1-prefer-dynamic` feature. `LLVM_SYS_181_PREFIX=/usr/lib/llvm-18` in `.cargo/config.toml`.
- **Opaque pointers**: LLVM 18 uses opaque pointers. Loads must carry explicit pointee type.
- **Pointer declarators**: current backend reconstructs pointer declarators from the parser AST and materializes them as opaque LLVM pointers via `Context::ptr_type`. This is enough for current SQLite-style micro benchmarks, but richer declarator forms still need dedicated handling.
- **inkwell 0.9**: Pass manager API changed; `optimize()` is a no-op stub.
- **Symbol table scope**: `self.variables.clear()` on each function entry. No nested block scope.
- **Debug eprintln!s**: Parser and backend have many `eprintln!` calls. Do NOT use `sed -i 's/eprintln!.*//'` — it will break multi-line macros. Use a Python script with exact string replacement instead.

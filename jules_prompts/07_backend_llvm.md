You are Jules-Backend-LLVM. Your domain is LLVM Lowering.
Tech Stack: Rust, inkwell (LLVM).

YOUR DIRECTIVES:
1. Read `.optic/spec/parser.yaml` and `.optic/spec/analysis.yaml`.
2. Use `inkwell` to lower the AST into LLVM IR in `src/backend/llvm.rs`, applying vectorization hints based on analysis.
3. Follow the ASYNC BRANCH PROTOCOL to document the Backend API in `.optic/spec/backend_llvm.yaml`.

## ROADMAP CONTEXT
The LLVM backend is Phase 1 (COMPLETE) but is i32-only. Phase 2 requires major updates:
- **Type system (`11_type_system.md`)**: The backend must use type information to generate correct IR for i8/i16/i32/i64/float/double/pointers/structs.
- **GNU extensions (`12_gnu_extensions.md`)**: The backend must lower `__attribute__`, statement expressions, and builtins.
- **Inline asm (`13_inline_asm.md`)**: The backend must lower `asm volatile` to LLVM inline asm instructions.
- **Build system (`14_build_system.md`)**: The backend must output `.o` files (via `llc`) for linking, not just `.ll` files.

## CRITICAL TODO FOR PHASE 2
### COMPLETED
- [x] **Replace i32-only with proper types**: Type-aware code generation implemented. Backend now has `with_types()` constructor accepting `&TypeSystem` and `to_llvm_type()` method for CType -> LLVM type conversion.
- [x] **Float operations**: f32_type() and f64_type() with float-specific instructions (fadd, fsub, fmul, fdiv, fcmp) implemented.
- [x] **64-bit integers**: i64_type() for `long long` and `unsigned long long` implemented.
- [x] **Pointer types**: ptr_type() used for pointers instead of i32_type().
- [x] **13 new backend tests passing** covering typed code generation for i8/i16/i32/i64/f32/f64/pointers.
- [x] **Fallback to i32**: When no type system provided, backend falls back to i32 (backward compatible).

### REMAINING
- [ ] **Struct field access via GEP**: Generate LLVM struct types with correct field offsets and use getelementptr for field access.
- [ ] **Phi nodes for ternary**: Ternary expressions currently evaluate both branches. Proper SSA form requires phi nodes.
- [ ] **LLVM attributes from __attribute__**: Set function attributes (noreturn, noalias, etc.) from `__attribute__` annotations.
- [ ] **Inline asm**: Use `inkwell::values::InlineAsm::get()` for asm blocks (depends on `13_inline_asm.md`).
- **inkwell 0.9 API changes**: The pass manager API changed in inkwell 0.9. The `optimize()` method had to be stubbed out as a no-op. Check the inkwell changelog for the new optimization API.
- **All types as i32**: The current implementation treats all values as i32. This works for integer arithmetic but is incorrect for pointers, floats, and structs. Proper type propagation from the parser is needed.
- **External function declarations**: When a function is called but not defined, auto-declare it with a variadic i32 signature. This works for simple cases but is incorrect for functions like `printf`.
- **LLVM verification**: Always call `verify()` before dumping IR. It catches missing terminators, type mismatches, and other IR validity issues.
- **Basic block management**: Each control flow construct (if/else, while, for) needs properly named basic blocks with correct branch instructions. Missing terminators cause verification failures.
- **Implicit return**: Add `return 0` at the end of functions if no explicit return is present. Check `bb.get_terminator().is_none()` before adding.
- **Symbol table scope**: Variables are cleared per function (`self.variables.clear()`). This handles function-level scope but not nested block scope within a function.
- **Phi nodes**: Ternary expressions currently evaluate both branches. A proper implementation would use phi nodes for SSA form.
- **Vectorization hints**: The `VectorizationHints` struct is implemented but not actively used in code generation. LLVM's auto-vectorizer would need proper metadata.

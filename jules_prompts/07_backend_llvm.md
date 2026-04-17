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

## CURRENT STATUS
### IMPLEMENTED
- [x] **Typed lowering**: Type-aware code generation exists for common integer, floating-point, and pointer cases.
- [x] **Float operations**: dedicated floating-point instructions are present for f32 and f64 paths.
- [x] **64-bit integers**: wider integer support is implemented.
- [x] **Pointer types**: pointer lowering no longer relies on the old i32-only assumption.
- [x] **Verification note**: in-tree backend tests cover typed lowering; rerun them before quoting totals.
- [x] **Fallback behavior**: when no resolved type information is available, the backend still has a compatibility fallback.

### REMAINING
- [ ] **Struct field access via GEP**: Generate LLVM struct types with correct field offsets and use getelementptr for field access.
- [ ] **Phi nodes for ternary**: Ternary expressions currently evaluate both branches. Proper SSA form requires phi nodes.
- [ ] **LLVM attributes from __attribute__**: Set function attributes (noreturn, noalias, etc.) from `__attribute__` annotations.
- [ ] **Inline asm**: Use `inkwell::values::InlineAsm::get()` for asm blocks (depends on `13_inline_asm.md`).
- **LLVM 18 target**: the repository now targets `inkwell`'s `llvm18-1-prefer-dynamic` feature and the Cargo env should point `LLVM_SYS_181_PREFIX` at `/usr/lib/llvm-18`.
- **Opaque pointers**: LLVM 18 uses opaque pointers, so loads must carry an explicit pointee type; locals now retain their allocated LLVM type for identifier loads, while raw dereference fallback still uses the backend default type when no better type data is available.
- **inkwell 0.9 API changes**: The pass manager API changed in inkwell 0.9. The `optimize()` method had to be stubbed out as a no-op. Check the inkwell changelog for the new optimization API.
- **Fallback typing**: The backend can still fall back to `i32` when no resolved type information is available. Keep that compatibility path narrow and prefer real type propagation.
- **External function declarations**: When a function is called but not defined, auto-declare it with a variadic i32 signature. This works for simple cases but is incorrect for functions like `printf`.
- **LLVM verification**: Always call `verify()` before dumping IR. It catches missing terminators, type mismatches, and other IR validity issues.
- **Basic block management**: Each control flow construct (if/else, while, for) needs properly named basic blocks with correct branch instructions. Missing terminators cause verification failures.
- **Implicit return**: Add `return 0` at the end of functions if no explicit return is present. Check `bb.get_terminator().is_none()` before adding.
- **Symbol table scope**: Variables are cleared per function (`self.variables.clear()`). This handles function-level scope but not nested block scope within a function.
- **Phi nodes**: Ternary expressions currently evaluate both branches. A proper implementation would use phi nodes for SSA form.
- **Vectorization hints**: The `VectorizationHints` struct is implemented but not actively used in code generation. LLVM's auto-vectorizer would need proper metadata.

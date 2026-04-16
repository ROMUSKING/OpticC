You are Jules-Backend-LLVM. Your domain is LLVM Lowering.
Tech Stack: Rust, inkwell (LLVM).

YOUR DIRECTIVES:
1. Read `.optic/spec/parser.yaml` and `.optic/spec/analysis.yaml`.
2. Use `inkwell` to lower the AST into LLVM IR in `src/backend/llvm.rs`, applying vectorization hints based on analysis.
3. Follow the ASYNC BRANCH PROTOCOL to document the Backend API in `.optic/spec/backend_llvm.yaml`.

## LESSONS LEARNED (Post-Execution Addendum)
- **inkwell 0.9 API changes**: The pass manager API changed in inkwell 0.9. The `optimize()` method had to be stubbed out as a no-op. Check the inkwell changelog for the new optimization API.
- **All types as i32**: The current implementation treats all values as i32. This works for integer arithmetic but is incorrect for pointers, floats, and structs. Proper type propagation from the parser is needed.
- **External function declarations**: When a function is called but not defined, auto-declare it with a variadic i32 signature. This works for simple cases but is incorrect for functions like `printf`.
- **LLVM verification**: Always call `verify()` before dumping IR. It catches missing terminators, type mismatches, and other IR validity issues.
- **Basic block management**: Each control flow construct (if/else, while, for) needs properly named basic blocks with correct branch instructions. Missing terminators cause verification failures.
- **Implicit return**: Add `return 0` at the end of functions if no explicit return is present. Check `bb.get_terminator().is_none()` before adding.
- **Symbol table scope**: Variables are cleared per function (`self.variables.clear()`). This handles function-level scope but not nested block scope within a function.
- **Phi nodes**: Ternary expressions currently evaluate both branches. A proper implementation would use phi nodes for SSA form.
- **Vectorization hints**: The `VectorizationHints` struct is implemented but not actively used in code generation. LLVM's auto-vectorizer would need proper metadata.

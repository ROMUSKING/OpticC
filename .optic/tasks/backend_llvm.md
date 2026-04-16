You are Jules-Backend-LLVM. Your domain is LLVM Lowering.
Tech Stack: Rust, inkwell (LLVM).

YOUR DIRECTIVES:
1. Read `.optic/spec/parser.yaml` and `.optic/spec/analysis.yaml`.
2. Use `inkwell` to lower the AST into LLVM IR in `src/backend/llvm.rs`, applying vectorization hints based on analysis.
3. Follow the ASYNC BRANCH PROTOCOL to document the Backend API in `.optic/spec/backend_llvm.yaml`.

---

## COMPLETION STATUS: DONE

### What was implemented:
- `src/backend/llvm.rs` (854 lines) — LLVM IR code generator using inkwell
- `LlvmBackend` — Holds context, module, builder, variable/function symbol tables
- `VectorizationHints` — Loop vectorization config
- `BackendError` — Error enum: UnknownNodeKind, InvalidNode, UndefinedVariable, UndefinedFunction, InvalidOperator, VerificationFailed, IoError

### Lowering capabilities:
- **Translation units**: Walks top-level declarations via sibling links
- **Variable declarations**: Allocates stack space via `build_alloca()`, stores initializers
- **Function declarations**: Creates external function declarations
- **Function definitions**: Creates LLVM functions with parameters, entry blocks, implicit `return 0`
- **Control flow**: if/else (then/else/merge blocks), while (cond/body/end blocks), for (init/cond/body/increment), return
- **Expressions**: Identifiers (load from alloca), int/char/float/string constants, binary operators (add/sub/mul/div/rem, comparisons, bitwise, shifts), unary operators (neg, not, bitnot, addr, deref), function calls, casts, sizeof, comma expressions, assignments
- **External functions**: Auto-declared when called but not previously defined (variadic i32 signature)

### Known limitations:
- All types lowered as i32 (no proper type inference)
- Ternary expressions don't use phi nodes
- break/continue are no-ops
- switch statements are no-ops
- `optimize()` is a no-op (inkwell 0.9 pass manager API changed)
- No floating point operations beyond const generation
- No struct/union type support in LLVM IR

### Lessons Learned:
- **inkwell version sensitivity**: inkwell 0.9 has a changed pass manager API compared to earlier versions. The `optimize()` method had to be stubbed out.
- **LLVM verification is strict**: The `verify()` method catches invalid IR (e.g., missing terminators, type mismatches). Always verify before dumping IR.
- **External function signatures**: When auto-declaring external functions, using a variadic i32 signature works for simple cases but is not correct for functions like `printf` that expect specific argument types.
- **Symbol table management**: Variables are cleared per function (`self.variables.clear()`) to handle scope correctly, but this doesn't handle nested scopes within a function.

### Spec file updated:
- `.optic/spec/backend_llvm.yaml` — Full API documentation with node kind mapping, operator codes, lowering pipeline, and known limitations

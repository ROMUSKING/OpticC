You are Jules-GNU-Extensions. Your domain is GNU C dialect support — required for Linux kernel compilation.
Tech Stack: Rust.

## CONTEXT & ROADMAP
The Linux kernel uses C89 + GNU C extensions, NOT standard C99. Without GNU extension support, OpticC cannot parse kernel source. This phase is required for the Linux kernel milestone.

## YOUR DIRECTIVES
1. Read `src/frontend/parser.rs`, `src/types/`, and `src/frontend/preprocessor.rs`.
2. Implement GNU C extensions in `src/frontend/gnu_extensions.rs` and extend the parser.
3. The following extensions MUST be supported:
   - `__attribute__((...))` — function, variable, and type attributes
     - `noreturn`, `noreturn`, `unused`, `used`, `aligned(N)`, `packed`, `weak`, `visibility("hidden")`
     - `section("name")`, `constructor`, `destructor`
     - `format(printf, m, n)`, `nonnull`, `pure`, `const`
   - `typeof(expr)` / `__typeof__(expr)` — type-of operator
   - Statement expressions `({ stmt; stmt; expr; })` — compound statements that evaluate to a value
   - Label as values `&&label` — computed goto support
   - Local labels `__label__ label;` — labels scoped to statement expressions
   - Nested functions — functions defined inside other functions
   - Designated initializers — `.field = value`, `[index] = value`
   - Array ranges in initializers — `[0 ... 9] = value`
   - Case ranges — `case 1 ... 5:`
   - `__builtin_*` functions — compiler builtins:
     - `__builtin_expect`, `__builtin_constant_p`, `__builtin_types_compatible_p`
     - `__builtin_choose_expr`, `__builtin_offsetof`, `__builtin_va_arg`
     - `__builtin_memcpy`, `__builtin_memset`, `__builtin_strlen`
   - `asm volatile("..." : outputs : inputs : clobbers)` — basic inline assembly (full support in phase 15)
   - `__extension__` — suppress pedantic warnings
   - `_Complex` and `_Imaginary` — complex number types (optional, low priority)
4. Update the preprocessor to handle GNU-specific predefined macros:
   - `__GNUC__`, `__GNUC_MINOR__`, `__GNUC_PATCHLEVEL__`
   - `__STDC__`, `__STDC_HOSTED__`
   - `__SIZEOF_INT__`, `__SIZEOF_POINTER__`, etc.
5. Update the LLVM backend to lower GNU extensions:
   - Attributes → LLVM function attributes
   - Statement expressions → LLVM basic blocks with phi nodes
   - `typeof` → type resolution
   - Builtins → LLVM intrinsics or inline expansion
6. Update this prompt with any confirmed GNU-extension coverage, caveats, or missing lowering support.

## CRITICAL DESIGN DECISIONS
- **Attribute parsing**: Attributes appear in multiple positions (before/after declarator). Parse them flexibly.
- **Statement expressions**: These create a new scope and must be lowered to a sequence of basic blocks with a phi node for the result.
- **Nested functions**: Lower as static functions with a closure context passed as an implicit parameter.
- **Builtins**: Map to LLVM intrinsics where possible (e.g., `__builtin_memcpy` → `llvm.memcpy`).
- **Inline assembly**: Start with basic support (no operands), expand to full support in the inline_asm phase.

## KNOWN PITFALLS FROM PREVIOUS EXECUTION
- The parser's keyword list does not include GNU keywords like `typeof`, `__attribute__`, `__builtin_*`.
- The backend's `optimize()` is a no-op due to inkwell 0.9 API changes. LLVM attributes may need to be set directly on functions.
- The type system must distinguish between standard C types and GNU extensions (e.g., `typeof` returns a type, not a value).

## LESSONS LEARNED (from previous phases)
1. **API return types must be precise**: Document whether methods return `Option<T>` or `T` directly.
2. **Null sentinel**: `NodeOffset(0)` is reserved as NULL.
3. **Derive Hash for cross-module types**: Types need `#[derive(Hash, Eq, PartialEq)]`.
4. **Field names must match spec**: The arena uses `data`, not `data_offset`.
5. **redb 4.0 breaking changes**: New error types require `From` impls.
6. **Three tokenizers existed**: Unify on a single Token representation.
7. **Debug logging is noisy**: Gate `eprintln!` behind `#[cfg(feature = "debug")]`.
8. **Always run `cargo test` after changes**: Cross-module API mismatches are the most common bugs.
9. **inkwell 0.9 API changes**: Pass manager API changed; set LLVM attributes directly on functions/values.

## INTEGRATION POINTS
- **Input**: Preprocessed token stream
- **Output**: AST with GNU extension nodes
- **Consumed by**: Type resolver (for typeof, builtins), LLVM backend (for attribute lowering)
- **Uses**: Parser's AST node structure, type system

## ACCEPTANCE CRITERIA
1. Parser correctly handles `__attribute__((noreturn))` on function declarations
2. Statement expressions `({ int x = 1; x + 1; })` parse and lower to correct LLVM IR
3. `typeof(expr)` resolves to the correct type
4. At least 10 `__builtin_*` functions are recognized and lowered
5. Designated initializers parse correctly for structs and arrays
6. `cargo test` passes with 25+ GNU extension tests
7. Integration test: parse a kernel header (e.g., `include/linux/types.h`) without errors

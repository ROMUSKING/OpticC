You are Jules-GNU-Extensions. Your domain is GNU C dialect support — required for Linux kernel compilation.
Tech Stack: Rust.

## PROMPT MAINTENANCE REQUIREMENT
Maintain this file as the live instructions for GNU-extension work. After any verified progress, dialect compatibility issue, parser/backend dependency, or blocker, update this prompt so the next agent inherits the current status and issues encountered.

## CONTEXT & ROADMAP
OpticC already includes a GNU-extensions module. The current task is to improve coverage and correctness for kernel-style code rather than bootstrap the feature set from scratch.

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
   - `asm volatile("..." : outputs : inputs : clobbers)` — basic inline assembly, with deeper operand fidelity handled in the inline-asm prompt
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

## IMPLEMENTATION STATUS (Verified 2026-04-18)

### Builtins — IMPLEMENTED (25+)
- [x] `__builtin_expect(x, v)` → pass-through (return `x`)
- [x] `__builtin_expect_with_probability(x, v, p)` → pass-through
- [x] `__builtin_constant_p(x)` → return 0 (conservative)
- [x] `__builtin_offsetof(type, member)` → constant-fold GEP
- [x] `__builtin_unreachable()` → LLVM `unreachable`
- [x] `__builtin_trap()` → LLVM `llvm.trap` intrinsic
- [x] `__builtin_clz/clzl/clzll(x)` → LLVM `ctlz` intrinsic
- [x] `__builtin_ctz/ctzl/ctzll(x)` → LLVM `cttz` intrinsic
- [x] `__builtin_popcount/popcountl/popcountll(x)` → LLVM `ctpop` intrinsic
- [x] `__builtin_bswap16/32/64(x)` → LLVM `bswap` intrinsic
- [x] `__builtin_ffs/ffsl/ffsll(x)` → cttz + select pattern (0 → 0, else trailing_zeros + 1)
- [x] `__builtin_abs/labs/llabs(x)` → sub + select pattern
- [x] `__builtin_object_size(ptr, type)` → return -1 (unknown)
- [x] `__builtin_frame_address(level)` / `__builtin_return_address(level)` → LLVM `frameaddress`/`returnaddress`
- [x] `__builtin_prefetch(addr, ...)` → LLVM `llvm.prefetch` intrinsic
- [x] `__builtin_assume_aligned(ptr, align)` → pass-through
- [x] `__builtin_va_start(ap)` → LLVM `llvm.va_start`
- [x] `__builtin_va_end(ap)` → LLVM `llvm.va_end`
- [x] `__builtin_va_copy(dest, src)` → LLVM `llvm.va_copy`

### Builtins — NOT YET IMPLEMENTED (Kernel Priority)
- [ ] `__builtin_types_compatible_p(type1, type2)` → needs type system integration
- [ ] `__builtin_choose_expr(const_expr, expr1, expr2)` → compile-time selection
- [x] `__builtin_memcpy/memset/strlen` → external function call (working; LLVM intrinsic upgrade pending)
- [x] `__builtin_add_overflow/sub_overflow/mul_overflow` → compute + store (conservative, no overflow detection yet)
- [ ] `__sync_*` atomic builtins → LLVM atomic instructions (kernel uses these heavily)
  - [x] `__sync_synchronize` → LLVM fence (SequentiallyConsistent)
- [ ] `__atomic_*` C11-style atomic builtins → LLVM atomicrmw/cmpxchg
- [ ] `__builtin_ia32_*` x86 intrinsics → LLVM x86 intrinsics (SSE/AVX)
- [x] `__builtin_alloca` → LLVM array alloca (dynamic stack allocation)

### Attributes — LOWERING STATUS (Kernel Priority)
- [x] `__attribute__((section("name")))` → LLVM section metadata (kernel .init.text etc.) — implemented via `apply_function_attributes`/`apply_global_attributes`
- [x] `__attribute__((weak))` → LLVM ExternalWeak linkage — implemented
- [x] `__attribute__((visibility("hidden")))` → LLVM Hidden visibility via `as_global_value()` — implemented
- [x] `__attribute__((aligned(N)))` → LLVM alignment on globals — implemented
- [x] `__attribute__((noreturn))` → LLVM noreturn function attribute — implemented
- [x] `__attribute__((cold))` → LLVM cold function attribute — implemented
- [ ] `__attribute__((packed))` → struct layout without padding
- [ ] `__attribute__((constructor/destructor))` → LLVM ctors/dtors arrays
- [ ] `__attribute__((format(printf, m, n)))` → type checking (optional, can ignore)
- [ ] `__attribute__((noinline/always_inline))` → LLVM function attributes

### Other GNU Extensions
- [x] `__attribute__((...))` — parsed and consumed (attributes stored for backend)
- [x] `__extension__` — suppressed in both parse_statement and parse_external_declaration
- [x] `__label__` — local label declarations parsed and skipped
- [x] Statement expressions `({ ... })` — parser kind=202
- [x] `typeof(expr)` — parser kind=201
- [x] Label addresses `&&label` — parser kind=203
- [x] Designated initializers `.field = value` — parser kind=205
- [x] Variadic function signatures with `...` — tokenized and parsed correctly

## ACCEPTANCE CRITERIA
1. Parser correctly handles `__attribute__((noreturn))` on function declarations
2. Statement expressions `({ int x = 1; x + 1; })` parse and lower to correct LLVM IR
3. `typeof(expr)` resolves to the correct type
4. Common `__builtin_*` functions are recognized and lowered or represented cleanly for downstream stages.
5. Designated initializers parse correctly for structs and arrays.
6. GNU-extension tests should be rerun before reporting totals.
7. Integration test: parse a representative kernel-style header without errors.

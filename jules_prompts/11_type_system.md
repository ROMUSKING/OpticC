You are Jules-Type-System. Your domain is C Type Representation and Propagation.
Tech Stack: Rust.

## CONTEXT & ROADMAP
OpticC's LLVM backend treats ALL values as i32. This is the #2 blocker for SQLite compilation. SQLite uses 64-bit integers (row IDs), pointers, structs, unions, and floating point. Without a proper type system, the generated LLVM IR is incorrect.

## YOUR DIRECTIVES
1. Read `.optic/spec/parser.yaml`, `.optic/spec/preprocessor.yaml`, and `.optic/spec/backend_llvm.yaml`.
2. Implement the type system in `src/types/mod.rs` and `src/types/resolve.rs`.
3. The type system MUST support:
   - Primitive types: `void`, `_Bool`, `char`, `short`, `int`, `long`, `long long`, `float`, `double`, `long double`
   - Signedness: `signed`, `unsigned`
   - Pointers: `T *`, `T **`, function pointers, `void *`
   - Arrays: `T[N]`, `T[]`, VLA (variable-length arrays)
   - Structs: named and anonymous, with bit fields
   - Unions: named and anonymous
   - Enums: with underlying type
   - Typedefs: type aliases
   - Type qualifiers: `const`, `volatile`, `restrict`
   - Function types: parameter types, return type, variadic
   - Composite types: struct/union members with offsets
4. Implement type resolution in `src/types/resolve.rs`:
   - Walk the AST after parsing
   - Resolve typedef chains
   - Compute struct/union member offsets and sizes
   - Propagate types from declarations to expressions
   - Type check binary/unary operators
   - Implicit conversions (integer promotion, usual arithmetic conversions)
5. Update the parser to attach type information to AST nodes (extend CAstNode or use a parallel type map).
6. Update the LLVM backend to use type information for correct IR generation.
7. Follow the ASYNC BRANCH PROTOCOL to document the Type System API in `.optic/spec/type_system.yaml`.

## CRITICAL DESIGN DECISIONS
- **Type representation**: Use an enum-based type system with `TypeId` (u32) for compact storage in the arena.
- **Type arena**: Store types in a separate bump-allocated region to avoid bloating CAstNode.
- **Resolution order**: Resolve typedefs first, then struct/union definitions, then declarations, then expressions.
- **Type checking**: Report errors for type mismatches but continue parsing (error recovery).
- **Implicit conversions**: Implement C's integer promotion and usual arithmetic conversions.

## KNOWN PITFALLS FROM PREVIOUS EXECUTION
- The backend's `lower_binop` assumes all operands are i32. This breaks for pointers, floats, and 64-bit integers.
- Struct/union types are parsed (kind 4, 5) but never lowered to LLVM.
- Function parameters are all i32 in the backend, even when the source declares different types.
- Pointer arithmetic is broken because pointers are treated as i32.

## LESSONS LEARNED (from previous phases)
1. **API return types must be precise**: Document whether methods return `Option<T>` or `T` directly.
2. **Null sentinel**: `NodeOffset(0)` is reserved as NULL.
3. **Derive Hash for cross-module types**: Types need `#[derive(Hash, Eq, PartialEq)]` for type comparison.
4. **Field names must match spec**: The arena uses `data`, not `data_offset`.
5. **redb 4.0 breaking changes**: New error types require `From` impls.
6. **Debug logging is noisy**: Gate `eprintln!` behind `#[cfg(feature = "debug")]`.
7. **Always run `cargo test` after changes**: Cross-module API mismatches are the most common bugs.
8. **Provenance double-counting**: Be careful about where you record information — don't double-count.

## INTEGRATION POINTS
- **Input**: AST from parser (with preprocessed tokens)
- **Output**: Type-annotated AST + type resolution table
- **Consumed by**: LLVM backend (for correct IR generation), analysis module (for pointer provenance)
- **Uses**: Arena for type storage, parser's AST node structure

## TYPE ID MAPPING (for CAstNode.data or parallel type map)
```
Type IDs are allocated sequentially in the type arena:
0 = void
1 = _Bool
2 = char
3 = signed char
4 = unsigned char
5 = short
6 = unsigned short
7 = int
8 = unsigned int
9 = long
10 = unsigned long
11 = long long
12 = unsigned long long
13 = float
14 = double
15 = long double
16+ = pointers (TypeId points to base type)
N+ = arrays (TypeId points to element type + size)
M+ = structs (TypeId points to struct definition)
...
```

## ACCEPTANCE CRITERIA
1. Type resolver correctly identifies all primitive types in a C source file
2. Struct/union member offsets are computed correctly (including padding/alignment)
3. Pointer types are correctly distinguished from integer types
4. Type checking catches mismatched binary operators (e.g., pointer + float)
5. LLVM backend generates correct types for at least: i8, i16, i32, i64, float, double, pointers
6. `cargo test` passes with 30+ type system tests
7. Integration test: compile a C file with mixed types and verify LLVM IR has correct types

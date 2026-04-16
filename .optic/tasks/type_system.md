You are Jules-Type-System. Your domain is C Type Representation and Propagation.
Tech Stack: Rust.

YOUR DIRECTIVES:
1. Read `.optic/spec/parser.yaml`, `.optic/spec/preprocessor.yaml`, `.optic/spec/backend_llvm.yaml`, and `.optic/spec/type_system.yaml`.
2. Implement the type system in `src/types/mod.rs` and `src/types/resolve.rs`.
3. Support: all C99 primitive types, pointers, arrays, structs, unions, enums, typedefs, function types, type qualifiers.
4. Implement type resolution: typedef chains, struct/union member offsets, type propagation, type checking, implicit conversions.
5. Update the LLVM backend to use type information for correct IR generation.
6. Follow the ASYNC BRANCH PROTOCOL to update `.optic/spec/type_system.yaml`.

COMPLETION STATUS:
- [x] Created `src/types/mod.rs` with CType enum, TypeId constants, TypeSystem, TypeQualifiers
- [x] Created `src/types/resolve.rs` with TypeResolver, TypeError, type checking operations
- [x] Updated `src/lib.rs` to export `pub mod types;`
- [x] Implemented all C99 primitive types (void, bool, char, short, int, long, long long, float, double, long double)
- [x] Implemented signed/unsigned variants for integer types
- [x] Implemented pointer types with base type tracking
- [x] Implemented array types with optional size
- [x] Implemented struct/union types with automatic layout computation (padding, alignment, bit fields)
- [x] Implemented enum types with underlying type
- [x] Implemented function types with parameters and variadic support
- [x] Implemented typedef types with chain resolution
- [x] Implemented type qualifiers (const, volatile, restrict) via bitflags
- [x] Implemented integer promotion (char/short -> int)
- [x] Implemented usual arithmetic conversions (int -> long -> long long -> float -> double)
- [x] Implemented binary operator type checking (add, sub, mul, div, mod, comparisons, logical, bitwise, shifts)
- [x] Implemented unary operator type checking (neg, not, bitnot, addr-of, deref, inc/dec, sizeof)
- [x] Implemented assignment compatibility checking
- [x] Implemented implicit conversion checking
- [x] Implemented pointer arithmetic (pointer + int, pointer - pointer)
- [x] Updated `.optic/spec/type_system.yaml` with complete API documentation
- [x] 70 comprehensive tests passing (26 in mod.rs, 44 in resolve.rs)
- [x] Made inkwell dependency optional to allow testing without LLVM

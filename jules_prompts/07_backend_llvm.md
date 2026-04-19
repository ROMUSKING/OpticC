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
- [x] **Switch/case codegen**: Full `lower_switch_stmt` with LLVM `build_switch`, case value → BasicBlock mapping, default block handling, and fall-through semantics. Tested with end-to-end test.
- [x] **Goto/label codegen**: `lower_goto_stmt` and `lower_labeled_stmt` with forward-reference label resolution via `label_blocks` HashMap. Labels are resolved lazily — if the label hasn't been seen yet, a forward BasicBlock is created.
- [x] **Break/continue**: `lower_break_continue` with `break_stack` and `continue_stack`. Pushed by while/for loops and switch statements. For-loop continue jumps to increment block.
- [x] **25+ builtins**: `lower_builtin_call` handles __builtin_clz/ctz/popcount/bswap (LLVM ctlz/cttz/ctpop/bswap intrinsics), __builtin_ffs (cttz+select), __builtin_abs (sub+select), __builtin_unreachable/trap (LLVM unreachable/llvm.trap), __builtin_expect/constant_p/offsetof (pass-through/constant-fold), __builtin_object_size/frame_address/return_address/prefetch, __builtin_expect_with_probability/assume_aligned (pass-through).
- [x] **Variadic functions**: Parser detects `...` in parameter lists, stores is_variadic flag (data=1 on kind=9 func declarator). Backend reads this in lower_func_def and pre_register_func_def, passes to fn_type(). `va_start`/`va_end`/`va_copy` intercepted in lower_call_expr, emitted as LLVM intrinsics.
- [x] **Attribute lowering**: `extract_attributes` walks kind=200 children. `apply_function_attributes` handles weak (ExternalWeak linkage), section, visibility (Hidden/Protected via as_global_value), noreturn, cold. `apply_global_attributes` handles weak, section, aligned, visibility. Applied in both `pre_register_func_def` and `lower_func_def`.
- [x] **Block scope**: `scope_stack: Vec<HashMap<String, Option<VariableBinding>>>` field. `push_scope()`/`pop_scope()` bracket `lower_compound`. `insert_scoped_variable()` saves previous binding before overwriting, restores on pop.
- [x] **Platform macros**: Preprocessor `platform_fallback_macros()` provides __linux__, __x86_64__, __LP64__, __BYTE_ORDER__, __CHAR_BIT__, __SIZE_TYPE__, etc. when no system compiler detected.

### KEY AST LAYOUT (after parser fix, 2026-04-17)
The parser now chains child nodes entirely via first_child chains, not via next_sibling of parent nodes:
- `kind=21` (var_decl): `first_child = type_spec → kind=73(init_declarator)`. The init_declarator has `first_child=kind=60(name)`, `next_sibling=init_expr`.
- `kind=23` (func_def): `first_child = return_type_spec → kind=9(func_decl) → kind=40(body)`. The kind=9 has `first_child=kind=60(name) → kind=24(param1) → kind=24(param2)`.
- `kind=24` (param_decl): `first_child = type_spec → kind=60(name)`. Name is last node in first_child chain.
- `kind=69` (member_access): `first_child=base_expr`, `next_sibling=field_ident`. NOTE: next_sibling here is the field name, not a sibling statement.
- `kind=9.next_sibling` is safe for link_siblings (params not stored there anymore).

### REMAINING BUGS (blockers for real-world C — tested 2026-04-19)
- [x] **sizeof(type) returns 0**: Fixed. sizeof tokenized as Keyword not Punctuator.
- [x] **Ternary operator always returned RHS**: Fixed. coerce_to_bool + build_select.
- [x] **Comma operator returned first value**: Fixed. Return right operand.
- [x] **Do-while loop condition never evaluated**: Fixed. Condition stored as body.next_sibling.
- [x] **P0: Extern function signatures**: Fixed. lower_func_decl now extracts param types from prototypes instead of defaulting to variadic. kind=22 pre-registered in Pass 1.
- [x] **P0: Pointer array indexing type**: Fixed. lower_array_element_ptr checks variable binding's pointee_type; if pointer, uses ptr GEP element type instead of i32.
- [x] **P0: Call argument isolation**: Fixed. Parser wraps each call argument in kind=74 wrapper node, preventing expression-internal next_sibling chains from leaking into call argument traversal.
- [x] **P0: Struct pointer field types**: Fixed. register_struct_types_in_node walks member children for pointer declarators (kind=7) and uses ptr type instead of i32.
- [x] **P0: Struct field name extraction**: Fixed. collect_struct_field_names descends into pointer/array declarators to find identifiers.
- [x] **P0: Nested member access**: Fixed. lower_member_access_ptr supports chained arrow operators (e.g., head->next->value) via recursive base expression lowering and find_member_access_root_var.
- [x] **P1: Struct return types**: Fixed. specifier_to_llvm_type resolves struct specifiers (kind=4/5) to LLVM struct types. lower_return_stmt handles StructType returns via build_return with struct values.
- [x] **P1: Assignment expression comparison**: Fixed. lower_assign_expr loads back from lvalue after store, returning runtime instruction instead of compile-time constant. Prevents LLVM constant folding in `(x = 42) > 0`.
- [x] **P1: Multi-variable complex declarators**: Fixed. parse_declarator stores pointer depth in data field of single kind=7 node; declarator_llvm_type reads depth from data instead of walking next_sibling chain.
- [x] **P2: Bitfield struct members**: Fixed. Parser stores bit_width in kind=27 data field. Backend packs consecutive bitfields into single LLVM storage units. struct_gep_info tracks (gep_index, bit_offset, bit_width). Read: lshr+and. Write: and+shl+or+store.
- [x] **P2: Designated initializer codegen**: Fixed. lower_designated_init_into_struct does GEP+store per .field=value pair. Dispatched from lower_var_decl when init is kind=205 and variable is struct type.
- [x] **P2: Compound literals**: Fixed. Parser detects (type_name){init_list} in parse_cast_expression, creates kind=212 node. Backend lower_compound_literal does alloca+store+load for structs, scalars, and arrays.

### KERNEL-PATH NEXT STEPS (Phase 3, Milestones 6b–7)
- [x] **Inline asm codegen (M4)**: `lower_asm_stmt` implemented. Reads template from arena, builds constraint string from operand children, creates InlineAsm via `context.create_inline_asm()`, calls via `build_indirect_call()`, stores outputs to lvalue pointers. Handles volatile, memory/cc clobbers, readwrite operands.
- [x] **Computed goto (M5)**: `lower_label_addr` produces LLVM blockaddress via `BasicBlock::get_address()`. `lower_goto_stmt` handles computed goto (`goto *expr`) via `build_indirect_branch`. All known label_blocks passed as possible destinations.
- [x] **Case ranges (M5)**: `case 1 ... 5:` parsed as kind=54, expanded to multiple switch entries in `collect_switch_cases`. Capped at 256 entries per range.
- [x] **Attribute lowering (M6a)**: `extract_attributes` walks kind=200 AST children. `apply_function_attributes` handles weak/section/visibility/noreturn/cold. `apply_global_attributes` handles weak/section/aligned/visibility.
- [x] **Block scope (M6a)**: `scope_stack` field on LlvmBackend. `push_scope()`/`pop_scope()` in `lower_compound`. `insert_scoped_variable()` saves/restores overwritten bindings.
- [x] **Bitfield support (M6b)**: Implemented. Parser stores bit_width in kind=27 data field. Backend packs bitfields, uses shift/mask for read (lshr+and) and write (and+shl+or+store). struct_gep_info HashMap tracks per-field metadata.
- [x] **Designated initializers (M6b)**: Fixed. lower_designated_init_into_struct does GEP+store per .field=value pair. Field name stored in arena.get_string(NodeOffset(node.data)).
- [x] **Compound literals (M6b)**: Fixed. Parser creates kind=212 in parse_cast_expression. Backend lower_compound_literal does alloca+store+load for structs/scalars/arrays.
- [x] **Multi-TU compilation (M6c)**: Fixed. kind=20 extern void declarations now pre-registered in Pass 1. Builder temp dir collision fixed with atomic invocation ID. End-to-end compile→link→run verified.
- [x] **System include paths (M6c)**: Verified working. discover_default_include_paths() detects gcc/clang paths, falls back to /usr/include. add_include_path() for -I flag. define_macro() for -D flag.

### KERNEL CODEGEN REQUIREMENTS (M7–M11)
These backend features are required for Linux kernel compilation:

#### Atomic Instruction Codegen (M7)
- `__sync_fetch_and_add(ptr, val)` → `atomicrmw add ptr, val seq_cst`
- `__sync_fetch_and_sub/or/and/xor` → `atomicrmw sub/or/and/xor`
- `__sync_val_compare_and_swap(ptr, old, new)` → `cmpxchg ptr, old, new seq_cst seq_cst`
- `__sync_lock_test_and_set(ptr, val)` → `atomicrmw xchg ptr, val acquire`
- `__sync_lock_release(ptr)` → `store 0, ptr release`
- `__atomic_load_n(ptr, order)` → `load atomic ptr, <order>`
- `__atomic_store_n(ptr, val, order)` → `store atomic val, ptr <order>`
- `__atomic_exchange_n(ptr, val, order)` → `atomicrmw xchg ptr, val <order>`
- `__atomic_compare_exchange_n` → `cmpxchg` with success/failure ordering
- `__atomic_fetch_add/sub/and/or/xor` → `atomicrmw` with specified ordering
- Memory ordering map: `__ATOMIC_RELAXED`→Monotonic, `__ATOMIC_CONSUME`→Monotonic, `__ATOMIC_ACQUIRE`→Acquire, `__ATOMIC_RELEASE`→Release, `__ATOMIC_ACQ_REL`→AcquireRelease, `__ATOMIC_SEQ_CST`→SequentiallyConsistent

#### Packed Struct Types (M8)
- When `__attribute__((packed))` is present, create LLVM StructType with `isPacked=true`
- In `register_struct_types_in_node`, check for packed attribute → set packed flag
- In `compute_struct_layout`, suppress padding when packed

#### Function Attributes (M8)
- `noinline` → add `Attribute::NoInline` to function
- `always_inline` → add `Attribute::AlwaysInline` to function
- `hot` → add `Attribute::Hot` to function (already have `cold`)

#### Global Constructor/Destructor Arrays (M8)
- `__attribute__((constructor(priority)))` → add function to `@llvm.global_ctors` array
- `__attribute__((destructor(priority)))` → add function to `@llvm.global_dtors` array
- Format: `@llvm.global_ctors = appending global [N x { i32, ptr, ptr }] [{ i32 priority, ptr @func, ptr null }]`

#### Kernel Code Model (M11)
- `-mcmodel=kernel` → set `CodeModel::Kernel` on LLVM TargetMachine
- This places code in the high address space (above 0xFFFF800000000000)

#### Red Zone Disable (M11)
- `-mno-red-zone` → add `noredzone` attribute to ALL functions
- Required for kernel code (interrupts can clobber red zone)

#### Thread-Local Globals (M9)
- `_Thread_local` / `__thread` → set `thread_local` attribute on LLVM global variable
- Kernel uses per-CPU variables via thread-local storage

#### Section Control (M11)
- `-fdata-sections` → each global in its own `.data.<name>` section
- `-ffunction-sections` → each function in its own `.text.<name>` section
- Enables linker `--gc-sections` to eliminate unused code

#### Optimization Passes
- See `jules_prompts/18_optimization_passes.md` for pass pipeline details
- Currently `llc -O2` provides backend optimization during object generation

## KNOWN CAVEATS
- **LLVM 18 target**: Targets `inkwell`'s `llvm18-1-prefer-dynamic` feature. `LLVM_SYS_181_PREFIX=/usr/lib/llvm-18` in `.cargo/config.toml`.
- **Opaque pointers**: LLVM 18 uses opaque pointers. Loads must carry explicit pointee type.
- **Pointer declarators**: current backend reconstructs pointer declarators from the parser AST and materializes them as opaque LLVM pointers via `Context::ptr_type`. This is enough for current SQLite-style micro benchmarks, but richer declarator forms still need dedicated handling.
- **inkwell 0.9**: Pass manager API changed; `optimize()` is a no-op stub.
- **Symbol table scope**: Nested block scope implemented via `scope_stack`. Functions still clear variables on entry, but compound statements push/pop properly.
- **Debug eprintln!s**: Parser and backend have many `eprintln!` calls. Do NOT use `sed -i 's/eprintln!.*//'` — it will break multi-line macros. Use a Python script with exact string replacement instead.

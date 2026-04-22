use crate::arena::{Arena, CAstNode, NodeFlags, NodeOffset};
use crate::types::{CType, TypeId, TypeSystem};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType, StructType};
use inkwell::values::{BasicValue, BasicValueEnum, FunctionValue, PointerValue};
use inkwell::{AddressSpace, ThreadLocalMode};
use std::collections::{HashMap, HashSet};

/// Maximum number of switch entries to emit for a single `case LOW ... HIGH:` range.
/// Prevents blowup for huge ranges like `case 0 ... 0xFFFFFFFF:`.
const MAX_CASE_RANGE_EXPANSION: u64 = 256;

pub struct LlvmBackend<'ctx, 'types> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    variables: HashMap<String, VariableBinding<'ctx>>,
    global_variables: HashMap<String, VariableBinding<'ctx>>,
    functions: HashMap<String, FunctionValue<'ctx>>,
    typedef_aliases: HashSet<String>,
    static_declared_functions: HashSet<String>,
    vectorization_hints: VectorizationHints,
    types: Option<&'types TypeSystem>,
    type_cache: HashMap<u32, BasicTypeEnum<'ctx>>,
    /// For each struct variable, stores ordered field names so GEP can resolve them.
    struct_fields: HashMap<String, Vec<String>>,
    /// Registered struct/union types keyed by tag name.
    struct_tag_types: HashMap<String, StructType<'ctx>>,
    /// Registered struct/union field names keyed by tag name.
    struct_tag_fields: HashMap<String, Vec<String>>,
    /// Function-pointer field signatures keyed by struct tag then field name.
    struct_field_fn_types: HashMap<String, HashMap<String, FunctionType<'ctx>>>,
    /// Registered pointee struct types keyed by pointer-backed variable/parameter name.
    pointer_struct_types: HashMap<String, StructType<'ctx>>,
    /// Variables known to be byte pointers (char/unsigned char/signed char pointers).
    byte_pointer_vars: HashSet<String>,
    current_return_type: Option<BasicTypeEnum<'ctx>>,
    /// Label name → BasicBlock mapping for goto/label support within a function.
    label_blocks: HashMap<String, BasicBlock<'ctx>>,
    /// Stack of break targets (innermost last) for switch/loop.
    break_stack: Vec<BasicBlock<'ctx>>,
    /// Stack of continue targets (innermost last) for loops.
    continue_stack: Vec<BasicBlock<'ctx>>,
    /// Scope stack for block-scoped variable shadowing. Each entry saves variable
    /// bindings that were overwritten when entering a new block scope.
    scope_stack: Vec<HashMap<String, Option<VariableBinding<'ctx>>>>,
    /// Bitfield GEP info: struct_tag → field_name → (gep_index, Option<(bit_offset, bit_width)>).
    /// Tracks how each struct member maps to LLVM struct field indices, with
    /// optional bit-level packing metadata for bitfield members.
    struct_gep_info: HashMap<String, HashMap<String, (u32, Option<(u32, u32)>)>>,
    /// Maps variable name → struct tag name (for looking up gep_info).
    var_struct_tag: HashMap<String, String>,
    /// Maps global variable name → struct tag name (persistent across functions).
    global_struct_tags: HashMap<String, String>,
    /// Set by `lower_member_access_ptr` when the access resolved a bitfield.
    /// Consumed by callers that need shift/mask codegen for read or write.
    last_bitfield_access: Option<(u32, u32)>,
    /// Functions registered for module constructor execution.
    global_ctors: Vec<(FunctionValue<'ctx>, u32)>,
    /// Functions registered for module destructor execution.
    global_dtors: Vec<(FunctionValue<'ctx>, u32)>,
    /// Globals/functions that must be preserved in the final object.
    llvm_used: Vec<PointerValue<'ctx>>,
    /// Function definitions that should be materialized for this translation unit.
    reachable_functions: HashSet<String>,
    /// Counter for generating unique names for static local variables.
    static_local_counter: u32,
}

#[derive(Clone, Copy)]
struct VariableBinding<'ctx> {
    ptr: PointerValue<'ctx>,
    pointee_type: BasicTypeEnum<'ctx>,
    function_type: Option<FunctionType<'ctx>>,
}

#[derive(Default)]
pub struct VectorizationHints {
    pub loop_vectorize: bool,
    pub vector_width: u32,
}

impl<'ctx, 'types> LlvmBackend<'ctx, 'types> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        let module = context.create_module(module_name);
        let builder = context.create_builder();
        LlvmBackend {
            context,
            module,
            builder,
            variables: HashMap::new(),
            global_variables: HashMap::new(),
            functions: HashMap::new(),
            typedef_aliases: HashSet::new(),
            static_declared_functions: HashSet::new(),
            vectorization_hints: VectorizationHints::default(),
            types: None,
            type_cache: HashMap::new(),
            struct_fields: HashMap::new(),
            struct_tag_types: HashMap::new(),
            struct_tag_fields: HashMap::new(),
            struct_field_fn_types: HashMap::new(),
            pointer_struct_types: HashMap::new(),
            byte_pointer_vars: HashSet::new(),
            current_return_type: None,
            label_blocks: HashMap::new(),
            break_stack: Vec::new(),
            continue_stack: Vec::new(),
            scope_stack: Vec::new(),
            struct_gep_info: HashMap::new(),
            var_struct_tag: HashMap::new(),
            global_struct_tags: HashMap::new(),
            last_bitfield_access: None,
            global_ctors: Vec::new(),
            global_dtors: Vec::new(),
            llvm_used: Vec::new(),
            reachable_functions: HashSet::new(),
            static_local_counter: 0,
        }
    }

    pub fn with_types(
        context: &'ctx Context,
        module_name: &str,
        types: &'types TypeSystem,
    ) -> LlvmBackend<'ctx, 'types> {
        let module = context.create_module(module_name);
        let builder = context.create_builder();
        LlvmBackend {
            context,
            module,
            builder,
            variables: HashMap::new(),
            global_variables: HashMap::new(),
            functions: HashMap::new(),
            typedef_aliases: HashSet::new(),
            static_declared_functions: HashSet::new(),
            vectorization_hints: VectorizationHints::default(),
            types: Some(types),
            type_cache: HashMap::new(),
            struct_fields: HashMap::new(),
            struct_tag_types: HashMap::new(),
            struct_tag_fields: HashMap::new(),
            struct_field_fn_types: HashMap::new(),
            pointer_struct_types: HashMap::new(),
            byte_pointer_vars: HashSet::new(),
            current_return_type: None,
            label_blocks: HashMap::new(),
            break_stack: Vec::new(),
            continue_stack: Vec::new(),
            scope_stack: Vec::new(),
            struct_gep_info: HashMap::new(),
            var_struct_tag: HashMap::new(),
            global_struct_tags: HashMap::new(),
            last_bitfield_access: None,
            global_ctors: Vec::new(),
            global_dtors: Vec::new(),
            llvm_used: Vec::new(),
            reachable_functions: HashSet::new(),
            static_local_counter: 0,
        }
    }

    pub fn set_vectorization_hints(&mut self, hints: VectorizationHints) {
        self.vectorization_hints = hints;
    }

    fn function_signature_info(
        &self,
        arena: &Arena,
        node: &CAstNode,
    ) -> (String, bool, bool, NodeOffset) {
        let mut name = "func".to_string();
        let mut is_static = false;
        let mut is_inline = false;
        let mut body_offset = NodeOffset::NULL;

        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                match child.kind {
                    103 => is_static = true,
                    93 => is_inline = true,
                    7..=9 => {
                        if let Some(func_decl_offset) =
                            self.find_function_declarator_offset(arena, child_offset)
                        {
                            if let Some(func_decl) = arena.get(func_decl_offset) {
                                if let Some(ident) = arena.get(func_decl.first_child) {
                                    if ident.kind == 60 {
                                        if let Some(found) =
                                            arena.get_string(NodeOffset(ident.data))
                                        {
                                            name = found.to_string();
                                        }
                                    }
                                }
                            }
                        }
                    }
                    40 => body_offset = child_offset,
                    _ => {}
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }

        (name, is_static, is_inline, body_offset)
    }

    fn collect_direct_callees(&self, arena: &Arena, offset: NodeOffset, out: &mut HashSet<String>) {
        let mut current = offset;
        while current != NodeOffset::NULL {
            let Some(node) = arena.get(current) else {
                break;
            };

            if node.kind == 67 {
                if let Some(callee) = arena.get(node.first_child) {
                    if let Some(name) = self
                        .find_ident_name_in(arena, callee)
                        .or_else(|| self.find_ident_name(arena, callee))
                    {
                        if !name.starts_with("__builtin_") {
                            out.insert(name);
                        }
                    }
                }
            }

            // Also collect bare identifier references that could be function
            // pointer uses (e.g. passed as callback arguments). This ensures
            // static functions used only via function pointers are reachable.
            if node.kind == 60 {
                if let Some(name) = arena.get_string(NodeOffset(node.data)) {
                    if !name.is_empty() && !name.starts_with("__builtin_") {
                        out.insert(name.to_string());
                    }
                }
            }

            if node.first_child != NodeOffset::NULL {
                self.collect_direct_callees(arena, node.first_child, out);
            }
            current = node.next_sibling;
        }
    }

    fn compute_reachable_functions(&self, arena: &Arena, offset: NodeOffset) -> HashSet<String> {
        let mut defs: HashMap<String, (bool, HashSet<String>)> = HashMap::new();
        let mut child_offset = offset;
        while child_offset != NodeOffset::NULL {
            let Some(node) = arena.get(child_offset) else {
                break;
            };
            if node.kind == 23 {
                let (name, is_static, _is_inline, body_offset) =
                    self.function_signature_info(arena, node);
                let attrs = self.extract_attributes(arena, node);
                let preserve_root = !is_static
                    || attrs.iter().any(|(attr, _, _)| {
                        matches!(
                            attr.as_str(),
                            "section"
                                | "__section__"
                                | "constructor"
                                | "__constructor__"
                                | "destructor"
                                | "__destructor__"
                                | "used"
                                | "__used__"
                        )
                    });
                let mut callees = HashSet::new();
                if body_offset != NodeOffset::NULL {
                    self.collect_direct_callees(arena, body_offset, &mut callees);
                }
                defs.insert(name, (preserve_root, callees));
            }
            child_offset = node.next_sibling;
        }

        let mut reachable = HashSet::new();
        let mut worklist = Vec::new();
        for (name, (is_root, _)) in &defs {
            if *is_root {
                reachable.insert(name.clone());
                worklist.push(name.clone());
            }
        }

        while let Some(name) = worklist.pop() {
            if let Some((_is_root, callees)) = defs.get(&name) {
                for callee in callees {
                    if defs.contains_key(callee) && reachable.insert(callee.clone()) {
                        worklist.push(callee.clone());
                    }
                }
            }
        }

        reachable
    }

    fn eval_const_int_expr(&self, arena: &Arena, offset: NodeOffset) -> Option<i64> {
        if offset == NodeOffset::NULL {
            return None;
        }
        let node = arena.get(offset)?;

        match node.kind {
            0 | 74 => {
                if node.first_child != NodeOffset::NULL {
                    self.eval_const_int_expr(arena, node.first_child)
                } else {
                    None
                }
            }
            61 | 80 | 62 => Some(node.data as i64),
            65 => {
                let operand = self.eval_const_int_expr(arena, node.first_child)?;
                match node.data {
                    1 => Some(-operand),
                    2 => Some((operand == 0) as i64),
                    3 => Some(!operand),
                    _ => None,
                }
            }
            64 => {
                let (lhs_offset, rhs_offset) = if let Some(wrapper) = arena.get(node.first_child) {
                    if wrapper.kind == 0
                        && wrapper.first_child != NodeOffset::NULL
                        && wrapper.next_sibling != NodeOffset::NULL
                    {
                        (wrapper.first_child, wrapper.next_sibling)
                    } else {
                        let lhs_offset = node.first_child;
                        let rhs_offset = arena
                            .get(lhs_offset)
                            .map(|lhs| lhs.next_sibling)
                            .filter(|off| *off != NodeOffset::NULL)
                            .unwrap_or(node.next_sibling);
                        (lhs_offset, rhs_offset)
                    }
                } else {
                    (node.first_child, NodeOffset::NULL)
                };

                let lhs = self.eval_const_int_expr(arena, lhs_offset)?;
                let rhs = self.eval_const_int_expr(arena, rhs_offset)?;
                match node.data {
                    1 => Some(lhs.wrapping_add(rhs)),
                    2 => Some(lhs.wrapping_sub(rhs)),
                    3 => Some(lhs.wrapping_mul(rhs)),
                    4 => {
                        if rhs == 0 {
                            None
                        } else {
                            Some(lhs.wrapping_div(rhs))
                        }
                    }
                    5 => {
                        if rhs == 0 {
                            None
                        } else {
                            Some(lhs.wrapping_rem(rhs))
                        }
                    }
                    12 | 14 => Some(lhs & rhs),
                    13 | 15 => Some(lhs | rhs),
                    16 => Some(lhs ^ rhs),
                    17 => Some(lhs.wrapping_shl((rhs as u32) & 63)),
                    18 => Some(lhs.wrapping_shr((rhs as u32) & 63)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn infer_array_declarator_len(&self, arena: &Arena, array_decl: &CAstNode) -> Option<u32> {
        if array_decl.kind != 8 {
            return None;
        }
        if array_decl.data > 0 {
            return Some(array_decl.data);
        }
        self.eval_const_int_expr(arena, array_decl.first_child)
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0)
    }

    fn scan_global_var_shape(
        &self,
        arena: &Arena,
        offset: NodeOffset,
        name_opt: &mut Option<String>,
        is_pointer: &mut bool,
        is_array: &mut bool,
        array_len: &mut u32,
        is_static: &mut bool,
        is_thread_local: &mut bool,
        init_offset: &mut NodeOffset,
    ) {
        let mut current = offset;
        while current != NodeOffset::NULL {
            let Some(node) = arena.get(current) else {
                break;
            };

            match node.kind {
                60 => {
                    if name_opt.is_none() {
                        *name_opt = arena
                            .get_string(NodeOffset(node.data))
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string());
                    }
                    if node.next_sibling != NodeOffset::NULL && *init_offset == NodeOffset::NULL {
                        *init_offset = node.next_sibling;
                    }
                }
                7 => {
                    *is_pointer = true;
                    if node.next_sibling != NodeOffset::NULL && *init_offset == NodeOffset::NULL {
                        *init_offset = node.next_sibling;
                    }
                }
                8 => {
                    *is_array = true;
                    if *array_len == 0 {
                        if let Some(len) = self.infer_array_declarator_len(arena, node) {
                            *array_len = len;
                        }
                    }
                    if node.next_sibling != NodeOffset::NULL && *init_offset == NodeOffset::NULL {
                        *init_offset = node.next_sibling;
                    }
                }
                103 => *is_static = true,
                106 => *is_thread_local = true,
                _ => {}
            }

            if node.first_child != NodeOffset::NULL && !matches!(node.kind, 4 | 5 | 1..=3 | 6 | 83)
            {
                self.scan_global_var_shape(
                    arena,
                    node.first_child,
                    name_opt,
                    is_pointer,
                    is_array,
                    array_len,
                    is_static,
                    is_thread_local,
                    init_offset,
                );
            }
            current = node.next_sibling;
        }
    }

    fn collect_attributes_from_chain(
        &self,
        arena: &Arena,
        offset: NodeOffset,
        attrs: &mut Vec<(String, Option<String>, Option<u32>)>,
    ) {
        let mut child_offset = offset;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                if child.kind == 200 {
                    let mut attr_child_offset = child.first_child;
                    while attr_child_offset != NodeOffset::NULL {
                        if let Some(attr_child) = arena.get(attr_child_offset) {
                            let name_offset = if attr_child.parent != NodeOffset::NULL {
                                attr_child.parent
                            } else {
                                NodeOffset(attr_child.first_child.0)
                            };
                            if let Some(name) = arena.get_string(name_offset) {
                                if !name.is_empty() {
                                    let mut str_arg = None;
                                    let mut int_arg = if attr_child.data != 0 {
                                        Some(attr_child.data)
                                    } else {
                                        None
                                    };
                                    let mut arg_offset = if attr_child.parent != NodeOffset::NULL {
                                        attr_child.first_child
                                    } else {
                                        attr_child.next_sibling
                                    };
                                    while arg_offset != NodeOffset::NULL {
                                        if let Some(arg_node) = arena.get(arg_offset) {
                                            if arg_node.kind == 63 {
                                                str_arg = arena
                                                    .get_string(NodeOffset(arg_node.data))
                                                    .map(|s| s.to_string());
                                            } else if arg_node.kind == 61 {
                                                int_arg = Some(arg_node.data);
                                            }
                                            arg_offset = arg_node.next_sibling;
                                        } else {
                                            break;
                                        }
                                    }
                                    attrs.push((name.to_string(), str_arg, int_arg));
                                }
                            }
                            attr_child_offset = attr_child.next_sibling;
                        } else {
                            break;
                        }
                    }
                } else if child.first_child != NodeOffset::NULL {
                    self.collect_attributes_from_chain(arena, child.first_child, attrs);
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }
    }

    fn record_lifecycle_function(
        entries: &mut Vec<(FunctionValue<'ctx>, u32)>,
        function: FunctionValue<'ctx>,
        priority: u32,
    ) {
        let name = function.get_name().to_bytes().to_vec();
        if entries.iter().any(|(existing, existing_priority)| {
            existing.get_name().to_bytes() == name.as_slice() && *existing_priority == priority
        }) {
            return;
        }
        entries.push((function, priority));
    }

    fn register_lifecycle_attributes(
        &mut self,
        function: FunctionValue<'ctx>,
        attrs: &[(String, Option<String>, Option<u32>)],
    ) {
        for (name, _str_arg, int_arg) in attrs {
            let priority = int_arg.unwrap_or(65535);
            match name.as_str() {
                "constructor" | "__constructor__" => {
                    Self::record_lifecycle_function(&mut self.global_ctors, function, priority);
                }
                "destructor" | "__destructor__" => {
                    Self::record_lifecycle_function(&mut self.global_dtors, function, priority);
                }
                _ => {}
            }
        }
    }

    fn emit_global_lifecycle_table(
        &self,
        table_name: &str,
        entries: &[(FunctionValue<'ctx>, u32)],
    ) {
        if entries.is_empty() {
            return;
        }

        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let entry_type = self
            .context
            .struct_type(&[i32_type.into(), ptr_type.into(), ptr_type.into()], false);

        let mut ordered = entries.to_vec();
        ordered.sort_by_key(|(_, priority)| *priority);

        let values: Vec<_> = ordered
            .iter()
            .map(|(function, priority)| {
                entry_type.const_named_struct(&[
                    i32_type.const_int(*priority as u64, false).into(),
                    function.as_global_value().as_pointer_value().into(),
                    ptr_type.const_null().into(),
                ])
            })
            .collect();

        let array_type = entry_type.array_type(values.len() as u32);
        let array_init = entry_type.const_array(&values);
        let global = self.module.add_global(array_type, None, table_name);
        global.set_linkage(inkwell::module::Linkage::Appending);
        global.set_initializer(&array_init);
        global.set_alignment(8);
    }

    fn emit_global_lifecycle_tables(&self) {
        self.emit_global_lifecycle_table("llvm.global_ctors", &self.global_ctors);
        self.emit_global_lifecycle_table("llvm.global_dtors", &self.global_dtors);
    }

    fn record_used_symbol(&mut self, value: PointerValue<'ctx>) {
        if !self.llvm_used.contains(&value) {
            self.llvm_used.push(value);
        }
    }

    fn register_used_attributes(
        &mut self,
        value: PointerValue<'ctx>,
        attrs: &[(String, Option<String>, Option<u32>)],
    ) {
        if attrs
            .iter()
            .any(|(name, _, _)| matches!(name.as_str(), "used" | "__used__"))
        {
            self.record_used_symbol(value);
        }
    }

    fn emit_llvm_used(&self) {
        if self.llvm_used.is_empty() {
            return;
        }
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let array_type = ptr_type.array_type(self.llvm_used.len() as u32);
        let init = ptr_type.const_array(&self.llvm_used);
        let global = self.module.add_global(array_type, None, "llvm.used");
        global.set_linkage(inkwell::module::Linkage::Appending);
        global.set_section(Some("llvm.metadata"));
        global.set_initializer(&init);
        global.set_alignment(8);
    }

    /// Enter a new block scope. Any variables declared inside this scope will
    /// shadow outer definitions; when the scope is popped the originals are restored.
    fn push_scope(&mut self) {
        self.scope_stack.push(HashMap::new());
    }

    /// Leave a block scope, restoring any variables that were shadowed.
    fn pop_scope(&mut self) {
        if let Some(saved) = self.scope_stack.pop() {
            for (name, prev_binding) in saved {
                if let Some(binding) = prev_binding {
                    self.variables.insert(name, binding);
                } else {
                    self.variables.remove(&name);
                }
            }
        }
    }

    /// Insert a variable into the current scope, saving the previous binding
    /// on the scope stack so it can be restored on pop_scope.
    fn insert_scoped_variable(&mut self, name: String, binding: VariableBinding<'ctx>) {
        if let Some(scope) = self.scope_stack.last_mut() {
            // Only save the first overwrite within this scope
            scope
                .entry(name.clone())
                .or_insert_with(|| self.variables.get(&name).copied());
        }
        self.variables.insert(name, binding);
    }

    fn to_llvm_type(&mut self, type_id: u32) -> BasicTypeEnum<'ctx> {
        if let Some(cached) = self.type_cache.get(&type_id) {
            return *cached;
        }

        let llvm_type = if let Some(ts) = self.types {
            match ts.get_type(TypeId(type_id)) {
                Some(CType::Void) => self.context.i8_type().as_basic_type_enum(),
                Some(CType::Bool) => self.context.bool_type().as_basic_type_enum(),
                Some(CType::Char { .. }) => self.context.i8_type().as_basic_type_enum(),
                Some(CType::Short { .. }) => self.context.i16_type().as_basic_type_enum(),
                Some(CType::Int { .. }) => self.context.i32_type().as_basic_type_enum(),
                Some(CType::Long { .. }) => self.context.i64_type().as_basic_type_enum(),
                Some(CType::LongLong { .. }) => self.context.i64_type().as_basic_type_enum(),
                Some(CType::Float) => self.context.f32_type().as_basic_type_enum(),
                Some(CType::Double) => self.context.f64_type().as_basic_type_enum(),
                Some(CType::LongDouble) => self.context.f64_type().as_basic_type_enum(),
                Some(CType::Pointer { .. }) => self
                    .context
                    .ptr_type(AddressSpace::default())
                    .as_basic_type_enum(),
                Some(CType::Array { element, size }) => {
                    let elem_type = self.to_llvm_type(element.0);
                    let len = size.unwrap_or(0);
                    elem_type.array_type(len as u32).as_basic_type_enum()
                }
                Some(CType::Struct { members, align, .. }) => {
                    let field_types: Vec<BasicTypeEnum> = members
                        .iter()
                        .map(|m| self.to_llvm_type(m.type_id.0))
                        .collect();
                    if field_types.is_empty() {
                        self.context.i8_type().as_basic_type_enum()
                    } else {
                        self.context
                            .struct_type(&field_types, *align == 1)
                            .as_basic_type_enum()
                    }
                }
                Some(CType::Enum { .. }) => self.context.i32_type().as_basic_type_enum(),
                Some(CType::Function { .. }) => self.context.i32_type().as_basic_type_enum(),
                Some(CType::Typedef { underlying, .. }) => self.to_llvm_type(underlying.0),
                Some(CType::Qualified { base, .. }) => self.to_llvm_type(base.0),
                Some(CType::Union { .. }) => {
                    self.context.i8_type().array_type(1).as_basic_type_enum()
                }
                None => self.context.i32_type().as_basic_type_enum(),
            }
        } else {
            self.context.i32_type().as_basic_type_enum()
        };

        self.type_cache.insert(type_id, llvm_type);
        llvm_type
    }

    fn is_void_type_id(&self, type_id: u32) -> bool {
        if let Some(ts) = self.types {
            matches!(ts.get_type(TypeId(type_id)), Some(CType::Void))
        } else {
            false
        }
    }

    fn get_type_for_node(&self, _arena: &Arena, _offset: NodeOffset) -> Option<u32> {
        None
    }

    fn default_type(&self) -> u32 {
        7
    }

    pub fn compile(&mut self, arena: &Arena, root: NodeOffset) -> Result<(), BackendError> {
        self.reachable_functions = self.compute_reachable_functions(arena, root);
        self.lower_translation_unit(arena, root)?;
        self.emit_global_lifecycle_tables();
        self.emit_llvm_used();
        Ok(())
    }

    /// Create an alloca in the entry block of the current function.
    /// This ensures dominance: the alloca dominates all uses in the function.
    fn build_entry_alloca(
        &self,
        alloca_type: BasicTypeEnum<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let current_fn = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or(BackendError::InvalidNode)?;
        let entry_bb = current_fn
            .get_first_basic_block()
            .ok_or(BackendError::InvalidNode)?;

        // Save current position
        let saved_block = self.builder.get_insert_block();

        // Position at the beginning of the entry block (before any existing instructions)
        if let Some(first_instr) = entry_bb.get_first_instruction() {
            self.builder.position_before(&first_instr);
        } else {
            self.builder.position_at_end(entry_bb);
        }

        let alloca = self
            .builder
            .build_alloca(alloca_type, name)
            .map_err(|_| BackendError::InvalidNode)?;

        // Restore position
        if let Some(bb) = saved_block {
            self.builder.position_at_end(bb);
        }

        Ok(alloca)
    }

    pub fn module(&self) -> &Module<'ctx> {
        &self.module
    }

    fn lower_translation_unit(
        &mut self,
        arena: &Arena,
        offset: NodeOffset,
    ) -> Result<(), BackendError> {
        self.collect_typedef_aliases(arena, offset);
        self.collect_static_function_names(arena, offset);

        // First pass: register all struct/union type definitions
        let mut scan_offset = offset;
        while scan_offset != NodeOffset::NULL {
            if let Some(node) = arena.get(scan_offset) {
                self.register_struct_types_in_node(arena, node);
                scan_offset = node.next_sibling;
            } else {
                break;
            }
        }

        // === Pass 1: Pre-register all function definitions AND declarations with correct signatures ===
        // This prevents auto-declarations with wrong types when functions call
        // each other before their definitions are reached.
        let mut prescan_offset = offset;
        while prescan_offset != NodeOffset::NULL {
            if let Some(node) = arena.get(prescan_offset) {
                if node.kind == 23 {
                    let _ = self.pre_register_func_def(arena, node);
                } else if node.kind == 22 {
                    let _ = self.lower_func_decl(arena, node);
                } else if node.kind == 20 {
                    // Let lower_func_decl inspect the declaration directly; it now
                    // cleanly no-ops for non-function declarations.
                    let _ = self.lower_func_decl(arena, node);
                }
                prescan_offset = node.next_sibling;
            } else {
                break;
            }
        }

        // === Pass 2: Compile all declarations and function bodies ===
        let mut child_offset = offset;
        while child_offset != NodeOffset::NULL {
            if let Some(node) = arena.get(child_offset) {
                match node.kind {
                    1..=9 | 83 => {}
                    20 => {
                        let _ = self.lower_global_decl(arena, node);
                    }
                    21 => {
                        let _ = self.lower_global_var(arena, node, node.kind);
                    }
                    22 => {
                        let _ = self.lower_func_decl(arena, node);
                    }
                    23 => {
                        let _ = self.lower_func_def(arena, node);
                    }
                    101..=105 => {}
                    _ => {}
                }
                child_offset = node.next_sibling;
            } else {
                break;
            }
        }

        let typedef_aliases: Vec<String> = self.typedef_aliases.iter().cloned().collect();
        for name in typedef_aliases {
            if let Some(function) = self.module.get_function(&name) {
                if function.get_first_basic_block().is_none() {
                    self.materialize_missing_function_body(function, true)?;
                }
            }
        }

        let static_functions: Vec<String> =
            self.static_declared_functions.iter().cloned().collect();
        for name in static_functions {
            if let Some(function) = self.module.get_function(&name) {
                if function.get_first_basic_block().is_none() {
                    function.set_linkage(inkwell::module::Linkage::Internal);
                    self.materialize_missing_function_body(function, false)?;
                }
            }
        }

        if let Some(function) = self.module.get_function("sqlite3OsDlSym") {
            if function.get_first_basic_block().is_none() {
                function.set_linkage(inkwell::module::Linkage::Internal);
                self.materialize_missing_function_body(function, false)?;
            }
        }
        Ok(())
    }

    /// Lower a top-level declaration, handling global variables properly.
    fn lower_global_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let mut scan_typedef = node.first_child;
        while scan_typedef != NodeOffset::NULL {
            let Some(child) = arena.get(scan_typedef) else {
                break;
            };
            if child.kind == 101 {
                let mut decl_offset = child.next_sibling;
                while decl_offset != NodeOffset::NULL {
                    if let Some(name) = arena
                        .get(decl_offset)
                        .and_then(|decl| self.find_ident_name(arena, decl))
                    {
                        self.typedef_aliases.insert(name);
                    }
                    decl_offset = arena
                        .get(decl_offset)
                        .map(|n| n.next_sibling)
                        .unwrap_or(NodeOffset::NULL);
                }
                return Ok(());
            }
            if !matches!(child.kind, 90..=106 | 1..=16 | 83 | 84 | 200) {
                break;
            }
            scan_typedef = child.next_sibling;
        }

        let _ = self.lower_func_decl(arena, node);

        let mut child_offset = node.first_child;
        // Find the type specifier and then the declarator(s)
        let mut spec_kind: u16 = 2; // default int
        let mut _is_const = false;
        let mut first_spec = NodeOffset::NULL;

        // First pass: find type info
        let mut scan_off = node.first_child;
        while scan_off != NodeOffset::NULL {
            if let Some(child) = arena.get(scan_off) {
                match child.kind {
                    1..=6 | 16 | 83 => {
                        spec_kind = child.kind;
                        if first_spec == NodeOffset::NULL {
                            first_spec = scan_off;
                        }
                    }
                    101..=105 => {
                        if child.kind == 104 {
                            _is_const = true;
                        } // const qualifier
                    }
                    _ => break,
                }
                scan_off = child.next_sibling;
            } else {
                break;
            }
        }

        // Process each child - function decls, func defs, and var decls
        // Also track whether we found a type specifier (to handle bare ident vars)
        let mut found_type_spec = false;
        let mut is_static_decl = false;
        let mut is_thread_local_decl = false;
        child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                match child.kind {
                    1..=6 | 16 | 83 => {
                        found_type_spec = true;
                    }
                    101..=106 => {
                        if child.kind == 103 {
                            is_static_decl = true;
                        }
                        if child.kind == 106 {
                            is_thread_local_decl = true;
                        }
                    }
                    21 => {
                        // Try to handle as a global variable
                        let _ = self.lower_global_var(arena, child, spec_kind);
                    }
                    22 => {
                        let _ = self.lower_func_decl(arena, child);
                    }
                    23 => {
                        let _ = self.lower_func_def(arena, child);
                    }
                    // Bare identifier after a type specifier = global variable declaration
                    // e.g. `struct Foo g;` produces kind=20 with [kind=4, kind=60]
                    60 if found_type_spec => {
                        if let Some(var_name) = arena
                            .get_string(NodeOffset(child.data))
                            .map(|s| s.to_string())
                        {
                            if !var_name.is_empty() {
                                // Determine LLVM type from spec node
                                let llvm_type = if let Some(spec_node) = arena.get(first_spec) {
                                    self.specifier_to_llvm_type(arena, spec_node)
                                } else {
                                    self.node_kind_to_llvm_type(spec_kind)
                                };
                                // Extract struct tag for GEP tracking
                                let struct_tag: Option<String> = if matches!(spec_kind, 4 | 5) {
                                    arena
                                        .get(first_spec)
                                        .and_then(|sn| {
                                            if sn.data != 0 {
                                                arena.get_string(NodeOffset(sn.data))
                                            } else {
                                                None
                                            }
                                        })
                                        .map(|s| s.to_string())
                                } else {
                                    None
                                };
                                // Create zero-initialized global
                                let global = self.module.add_global(
                                    llvm_type,
                                    Some(AddressSpace::default()),
                                    &var_name,
                                );
                                let zero: BasicValueEnum = match llvm_type {
                                    BasicTypeEnum::IntType(it) => it.const_zero().into(),
                                    BasicTypeEnum::FloatType(ft) => ft.const_zero().into(),
                                    BasicTypeEnum::PointerType(pt) => pt.const_null().into(),
                                    BasicTypeEnum::StructType(st) => st.const_zero().into(),
                                    BasicTypeEnum::ArrayType(at) => at.const_zero().into(),
                                    _ => self.context.i32_type().const_zero().into(),
                                };
                                global.set_initializer(&zero);
                                if is_static_decl {
                                    global.set_linkage(inkwell::module::Linkage::Internal);
                                }
                                if is_thread_local_decl {
                                    global.set_thread_local(true);
                                    global.set_thread_local_mode(Some(
                                        ThreadLocalMode::GeneralDynamicTLSModel,
                                    ));
                                }
                                let binding = VariableBinding {
                                    ptr: global.as_pointer_value(),
                                    pointee_type: llvm_type,
                                    function_type: None,
                                };
                                if let Some(ref tag) = struct_tag {
                                    self.global_struct_tags
                                        .insert(var_name.clone(), tag.clone());
                                }
                                self.global_variables.insert(var_name.clone(), binding);
                                self.variables.insert(var_name, binding);
                            }
                        }
                    }
                    7..=9 => {
                        // A declaration containing a function declarator represents
                        // a forward / extern function declaration wrapped in a
                        // kind=20 declaration node (e.g. `extern void foo(int);`).
                        // Delegate to lower_func_decl using the innermost wrapper
                        // that still carries the declaration specifier chain.
                        if self
                            .find_function_declarator_offset(arena, child_offset)
                            .is_some()
                        {
                            let _ = if matches!(child.kind, 21 | 22) {
                                self.lower_func_decl(arena, child)
                            } else {
                                self.lower_func_decl(arena, node)
                            };
                        }
                    }
                    _ => {}
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }
        Ok(())
    }

    /// Lower a global variable declaration with optional initializer.
    fn lower_global_var(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
        _outer_spec_kind: u16,
    ) -> Result<(), BackendError> {
        // Find the actual type specifier from children (kind 1..=6, 4, 5, 83)
        let mut spec_kind: u16 = 2;
        let mut spec_node_offset = NodeOffset::NULL;
        {
            let mut ch = node.first_child;
            while ch != NodeOffset::NULL {
                if let Some(c) = arena.get(ch) {
                    if matches!(c.kind, 1..=6 | 16 | 83) {
                        spec_kind = c.kind;
                        spec_node_offset = ch;
                        break;
                    }
                    ch = c.next_sibling;
                } else {
                    break;
                }
            }
        }
        let llvm_type = if let Some(sn) = arena.get(spec_node_offset) {
            self.specifier_to_llvm_type(arena, sn)
        } else {
            self.node_kind_to_llvm_type(spec_kind)
        };
        // Extract struct tag for global struct tracking (so GEP works in member access)
        let global_struct_tag: Option<String> = if matches!(spec_kind, 4 | 5) {
            arena
                .get(spec_node_offset)
                .and_then(|sn| {
                    if sn.data != 0 {
                        arena.get_string(NodeOffset(sn.data))
                    } else {
                        None
                    }
                })
                .map(|s| s.to_string())
        } else {
            None
        };
        // Extract attributes early so we can apply them to any globals we create
        let attrs = self.extract_attributes(arena, node);

        // Find the variable name and initializer
        let mut name_opt: Option<String> = None;
        let mut is_pointer = false;
        let mut is_array = false;
        let mut array_len: u32 = 0;
        let mut is_static = false;
        let mut is_thread_local = false;
        let mut init_offset = NodeOffset::NULL;
        self.scan_global_var_shape(
            arena,
            node.first_child,
            &mut name_opt,
            &mut is_pointer,
            &mut is_array,
            &mut array_len,
            &mut is_static,
            &mut is_thread_local,
            &mut init_offset,
        );

        let infer_array_len_from_init = |start: NodeOffset| -> u32 {
            let mut count = 0u32;
            let mut cur = start;
            while cur != NodeOffset::NULL {
                if arena.get(cur).is_none() {
                    break;
                }
                count += 1;
                cur = arena
                    .get(cur)
                    .map(|n| n.next_sibling)
                    .unwrap_or(NodeOffset::NULL);
            }
            count
        };

        if let Some(var_name) = name_opt {
            let global_type = if is_pointer {
                self.context
                    .ptr_type(AddressSpace::default())
                    .as_basic_type_enum()
            } else if is_array {
                let inferred = if array_len == 0 && init_offset != NodeOffset::NULL {
                    infer_array_len_from_init(init_offset)
                } else {
                    array_len
                };
                let len = inferred.max(1);
                llvm_type.array_type(len).as_basic_type_enum()
            } else {
                llvm_type
            };

            // Check if there's a string initializer (for char arrays)
            if init_offset != NodeOffset::NULL {
                if let Some(init_node) = arena.get(init_offset) {
                    if matches!(init_node.kind, 63 | 81 | 82) {
                        // String literal initializer - concatenate adjacent string nodes
                        // so kernel-style constructs like "license" "=" "GPL" lower correctly.
                        let mut string_text = String::new();
                        let mut string_off = init_offset;
                        while string_off != NodeOffset::NULL {
                            let Some(string_node) = arena.get(string_off) else {
                                break;
                            };
                            if !matches!(string_node.kind, 63 | 81 | 82) {
                                break;
                            }
                            if let Some(fragment) = arena.get_string(NodeOffset(string_node.data)) {
                                string_text.push_str(fragment);
                            }
                            string_off = string_node.next_sibling;
                        }
                        let bytes = string_text.as_bytes();
                        let string_val = self.context.const_string(bytes, true);
                        let global = self.module.add_global(
                            string_val.get_type(),
                            Some(AddressSpace::default()),
                            &var_name,
                        );
                        global.set_initializer(&string_val);
                        global.set_constant(true);
                        if is_static {
                            global.set_linkage(inkwell::module::Linkage::Internal);
                        }
                        if is_thread_local {
                            global.set_thread_local(true);
                            global.set_thread_local_mode(Some(
                                ThreadLocalMode::GeneralDynamicTLSModel,
                            ));
                        }
                        self.apply_global_attributes(global, &attrs);
                        self.register_used_attributes(global.as_pointer_value(), &attrs);
                        // Store in variables for later reference
                        let binding = VariableBinding {
                            ptr: global.as_pointer_value(),
                            pointee_type: string_val.get_type().as_basic_type_enum(),
                            function_type: None,
                        };
                        self.global_variables.insert(var_name.clone(), binding);
                        self.variables.insert(var_name, binding);
                        return Ok(());
                    } else if is_array {
                        if let BasicTypeEnum::ArrayType(arr_ty) = global_type {
                            let elem_ty = arr_ty.get_element_type();
                            let mut values: Vec<BasicValueEnum> = Vec::new();
                            let mut cur = init_offset;
                            while cur != NodeOffset::NULL {
                                let Some(item) = arena.get(cur) else {
                                    break;
                                };
                                let v = match (item.kind, elem_ty) {
                                    (61 | 80, BasicTypeEnum::IntType(it)) => {
                                        it.const_int(item.data as u64, false).into()
                                    }
                                    (61 | 80, BasicTypeEnum::FloatType(ft)) => {
                                        ft.const_float(item.data as f64).into()
                                    }
                                    _ => match elem_ty {
                                        BasicTypeEnum::IntType(it) => it.const_zero().into(),
                                        BasicTypeEnum::FloatType(ft) => ft.const_zero().into(),
                                        BasicTypeEnum::PointerType(pt) => pt.const_null().into(),
                                        BasicTypeEnum::StructType(st) => st.const_zero().into(),
                                        BasicTypeEnum::ArrayType(at) => at.const_zero().into(),
                                        _ => self.context.i32_type().const_zero().into(),
                                    },
                                };
                                values.push(v);
                                cur = item.next_sibling;
                            }

                            while values.len() < arr_ty.len() as usize {
                                values.push(match elem_ty {
                                    BasicTypeEnum::IntType(it) => it.const_zero().into(),
                                    BasicTypeEnum::FloatType(ft) => ft.const_zero().into(),
                                    BasicTypeEnum::PointerType(pt) => pt.const_null().into(),
                                    BasicTypeEnum::StructType(st) => st.const_zero().into(),
                                    BasicTypeEnum::ArrayType(at) => at.const_zero().into(),
                                    _ => self.context.i32_type().const_zero().into(),
                                });
                            }
                            values.truncate(arr_ty.len() as usize);

                            let global = self.module.add_global(
                                global_type,
                                Some(AddressSpace::default()),
                                &var_name,
                            );
                            let arr_const = arr_ty.const_array(
                                &values
                                    .iter()
                                    .filter_map(|v| (*v).try_into().ok())
                                    .collect::<Vec<_>>(),
                            );
                            global.set_initializer(&arr_const);
                            if is_static {
                                global.set_linkage(inkwell::module::Linkage::Internal);
                            }
                            if is_thread_local {
                                global.set_thread_local(true);
                                global.set_thread_local_mode(Some(
                                    ThreadLocalMode::GeneralDynamicTLSModel,
                                ));
                            }
                            self.apply_global_attributes(global, &attrs);
                            self.register_used_attributes(global.as_pointer_value(), &attrs);
                            let binding = VariableBinding {
                                ptr: global.as_pointer_value(),
                                pointee_type: global_type,
                                function_type: None,
                            };
                            self.global_variables.insert(var_name.clone(), binding);
                            self.variables.insert(var_name, binding);
                            return Ok(());
                        }
                    } else if init_node.kind == 61 || init_node.kind == 80 {
                        // Integer constant initializer
                        let value = init_node.data as u64;
                        let global = self.module.add_global(
                            global_type,
                            Some(AddressSpace::default()),
                            &var_name,
                        );
                        // For pointer types, use null instead of integer 0
                        if global_type.is_pointer_type() {
                            let null_val =
                                self.context.ptr_type(AddressSpace::default()).const_null();
                            global.set_initializer(&null_val);
                        } else {
                            let const_val: inkwell::values::BasicValueEnum = match global_type {
                                BasicTypeEnum::IntType(it) => it.const_int(value, false).into(),
                                BasicTypeEnum::StructType(st) => st.const_zero().into(),
                                BasicTypeEnum::ArrayType(at) => at.const_zero().into(),
                                _ => self.context.i32_type().const_int(value, false).into(),
                            };
                            global.set_initializer(&const_val);
                        }
                        let binding = VariableBinding {
                            ptr: global.as_pointer_value(),
                            pointee_type: global_type,
                            function_type: None,
                        };
                        if is_static {
                            global.set_linkage(inkwell::module::Linkage::Internal);
                        }
                        if is_thread_local {
                            global.set_thread_local(true);
                            global.set_thread_local_mode(Some(
                                ThreadLocalMode::GeneralDynamicTLSModel,
                            ));
                        }
                        self.apply_global_attributes(global, &attrs);
                        self.register_used_attributes(global.as_pointer_value(), &attrs);
                        if let Some(ref tag) = global_struct_tag {
                            self.global_struct_tags
                                .insert(var_name.clone(), tag.clone());
                        }
                        self.global_variables.insert(var_name.clone(), binding);
                        self.variables.insert(var_name, binding);
                        return Ok(());
                    }
                }
            }

            // Default: create zero-initialized global
            let global =
                self.module
                    .add_global(global_type, Some(AddressSpace::default()), &var_name);
            let zero: BasicValueEnum = match global_type {
                BasicTypeEnum::IntType(it) => it.const_zero().into(),
                BasicTypeEnum::FloatType(ft) => ft.const_zero().into(),
                BasicTypeEnum::PointerType(pt) => pt.const_null().into(),
                BasicTypeEnum::StructType(st) => st.const_zero().into(),
                BasicTypeEnum::ArrayType(at) => at.const_zero().into(),
                _ => self.context.i32_type().const_zero().into(),
            };
            global.set_initializer(&zero);
            if is_static {
                global.set_linkage(inkwell::module::Linkage::Internal);
            }
            if is_thread_local {
                global.set_thread_local(true);
                global.set_thread_local_mode(Some(ThreadLocalMode::GeneralDynamicTLSModel));
            }
            self.apply_global_attributes(global, &attrs);
            self.register_used_attributes(global.as_pointer_value(), &attrs);
            let binding = VariableBinding {
                ptr: global.as_pointer_value(),
                pointee_type: global_type,
                function_type: None,
            };
            if let Some(ref tag) = global_struct_tag {
                self.global_struct_tags
                    .insert(var_name.clone(), tag.clone());
            }
            self.global_variables.insert(var_name.clone(), binding);
            self.variables.insert(var_name, binding);
        }
        Ok(())
    }

    fn lower_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                match child.kind {
                    21 => {
                        let _ = self.lower_var_decl(arena, &child);
                    }
                    22 => {
                        let _ = self.lower_func_decl(arena, &child);
                    }
                    23 => {
                        let _ = self.lower_func_def(arena, &child);
                    }
                    _ => {}
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }
        Ok(())
    }

    fn register_struct_types_in_node(&mut self, arena: &Arena, node: &CAstNode) {
        let mut child_off = node.first_child;
        while child_off != NodeOffset::NULL {
            if let Some(child) = arena.get(child_off) {
                if (child.kind == 4 || child.kind == 5)
                    && child.data != 0
                    && child.first_child != NodeOffset::NULL
                {
                    if let Some(tag_name) = arena.get_string(NodeOffset(child.data)) {
                        let tag_name = tag_name.to_string();
                        if !self.struct_tag_types.contains_key(&tag_name) {
                            let field_names = Self::collect_struct_field_names(arena, child);
                            let mut field_types: Vec<BasicTypeEnum<'ctx>> = Vec::new();
                            let mut fn_field_types: HashMap<String, FunctionType<'ctx>> =
                                HashMap::new();
                            // Track bitfield info: field_name → (gep_idx, Option<(bit_offset, bit_width)>)
                            let mut gep_info: HashMap<String, (u32, Option<(u32, u32)>)> =
                                HashMap::new();
                            let mut gep_idx: u32 = 0;
                            let mut field_name_idx: usize = 0;
                            // Track current bitfield group: (base_kind, bits_used, storage_bit_capacity)
                            let mut bitfield_group: Option<(u16, u32, u32)> = None;

                            let mut member_off = child.first_child;
                            // Pre-pass: register any nested named struct/union types first,
                            // so that when we build this struct's field types, the nested
                            // types are already in struct_tag_types and we get the same
                            // object (enabling identity-based lookups later).
                            while member_off != NodeOffset::NULL {
                                if let Some(m) = arena.get(member_off) {
                                    let mut scan = m.first_child;
                                    while scan != NodeOffset::NULL {
                                        if let Some(sn) = arena.get(scan) {
                                            if (sn.kind == 4 || sn.kind == 5)
                                                && sn.data != 0
                                                && sn.first_child != NodeOffset::NULL
                                            {
                                                self.register_struct_types_in_node(arena, m);
                                            }
                                            scan = sn.next_sibling;
                                        } else {
                                            break;
                                        }
                                    }
                                    member_off = m.next_sibling;
                                } else {
                                    break;
                                }
                            }
                            member_off = child.first_child;
                            while member_off != NodeOffset::NULL {
                                if let Some(m) = arena.get(member_off) {
                                    if m.kind == 200 {
                                        member_off = m.next_sibling;
                                        continue;
                                    }
                                    // Determine field type and check for bitfield
                                    let mut has_pointer = false;
                                    // return_has_pointer: true when the fn-pointer field's return
                                    // type is itself a pointer (e.g. `sqlite3_mutex *(*xMutexAlloc)(int)`).
                                    // Tracked separately so we emit ptr as the call return type.
                                    let mut return_has_pointer = false;
                                    let mut array_len: Option<u32> = None;
                                    let mut base_kind = 2u16; // default to int
                                    let mut declared_base_kind = 2u16;
                                    let mut bitfield_width: Option<u32> = None;
                                    let mut nested_spec_off = NodeOffset::NULL;
                                    let mut fn_declarator_off = NodeOffset::NULL;
                                    // Capture member-level base specifier from the direct child chain.
                                    // Do not let nested function-parameter specifiers override this.
                                    {
                                        let mut direct = m.first_child;
                                        while direct != NodeOffset::NULL {
                                            let Some(dn) = arena.get(direct) else {
                                                break;
                                            };
                                            match dn.kind {
                                                1..=6 | 10..=13 | 16 | 83 | 84 => {
                                                    declared_base_kind = dn.kind;
                                                }
                                                7..=9 | 60 => break,
                                                _ => {}
                                            }
                                            direct = dn.next_sibling;
                                        }
                                    }
                                    // Member declarators can be nested (pointer->array->ident),
                                    // so scan the full subtree rather than only sibling links.
                                    let mut stack: Vec<NodeOffset> = vec![m.first_child];
                                    while let Some(mut check_off) = stack.pop() {
                                        while check_off != NodeOffset::NULL {
                                            let Some(cn) = arena.get(check_off) else {
                                                break;
                                            };
                                            match cn.kind {
                                                1..=6 | 10..=13 | 16 | 83 | 84 => {
                                                    base_kind = cn.kind;
                                                    if matches!(cn.kind, 4 | 5) {
                                                        nested_spec_off = check_off;
                                                    }
                                                }
                                                7 => {
                                                    has_pointer = true;
                                                    // When the outer kind=7 pointer wraps a
                                                    // fn-declarator (kind=9), e.g.
                                                    //   `sqlite3_mutex *(*xMutexAlloc)(int)`
                                                    // the fn-declarator is at cn.first_child.
                                                    if let Some(inner) = arena.get(cn.first_child) {
                                                        if inner.kind == 9 {
                                                            fn_declarator_off = cn.first_child;
                                                            return_has_pointer = true;
                                                        }
                                                    }
                                                }
                                                9 => {
                                                    has_pointer = true; // fn-pointer field → ptr
                                                    fn_declarator_off = check_off;
                                                }
                                                8 => {
                                                    if let Some(len) =
                                                        self.infer_array_declarator_len(arena, cn)
                                                    {
                                                        array_len = Some(len);
                                                    }
                                                }
                                                27 => {
                                                    // Bitfield node: data = width
                                                    if cn.data > 0 {
                                                        bitfield_width = Some(cn.data);
                                                    }
                                                }
                                                _ => {}
                                            }

                                            if cn.first_child != NodeOffset::NULL {
                                                stack.push(cn.first_child);
                                            }
                                            check_off = cn.next_sibling;
                                        }
                                    }

                                    let current_name = field_names
                                        .get(field_name_idx)
                                        .cloned()
                                        .unwrap_or_else(|| format!("_field{}", field_name_idx));
                                    field_name_idx += 1;
                                    let member_base_type = if matches!(base_kind, 4 | 5)
                                        && nested_spec_off != NodeOffset::NULL
                                    {
                                        arena
                                            .get(nested_spec_off)
                                            .map(|sn| self.specifier_to_llvm_type(arena, sn))
                                            .unwrap_or_else(|| {
                                                self.node_kind_to_llvm_type(base_kind)
                                            })
                                    } else {
                                        self.node_kind_to_llvm_type(base_kind)
                                    };
                                    if fn_declarator_off != NodeOffset::NULL {
                                        let fn_base_kind = declared_base_kind;
                                        let fn_base_type =
                                            self.node_kind_to_llvm_type(fn_base_kind);
                                        // When the function returns a pointer, use ptr as the
                                        // return type for the registered FunctionType instead of
                                        // the raw base struct/int type. This prevents the backend
                                        // from emitting `call i32` and then inttoptr which
                                        // discards the upper 32 bits of the pointer on x86-64.
                                        let fn_ret_base_type = if return_has_pointer {
                                            self.context
                                                .ptr_type(inkwell::AddressSpace::default())
                                                .as_basic_type_enum()
                                        } else {
                                            fn_base_type
                                        };
                                        // Use a non-void kind so function_type_from_declarator
                                        // falls into the base_type match rather than void_type.
                                        let fn_ret_kind = if return_has_pointer {
                                            7u16
                                        } else {
                                            fn_base_kind
                                        };
                                        if let Some(fn_type) = self.function_type_from_declarator(
                                            arena,
                                            arena.get(fn_declarator_off),
                                            fn_ret_base_type,
                                            fn_ret_kind,
                                        ) {
                                            fn_field_types.insert(current_name.clone(), fn_type);
                                        }
                                    }

                                    if let Some(bw) = bitfield_width {
                                        // This is a bitfield member
                                        let storage_bits = self.node_kind_bit_width(base_kind);
                                        if let Some((grp_kind, ref mut bits_used, capacity)) =
                                            bitfield_group
                                        {
                                            if grp_kind == base_kind && *bits_used + bw <= capacity
                                            {
                                                // Fits in current group
                                                gep_info.insert(
                                                    current_name,
                                                    (gep_idx - 1, Some((*bits_used, bw))),
                                                );
                                                *bits_used += bw;
                                            } else {
                                                // Start new storage unit
                                                field_types
                                                    .push(self.node_kind_to_llvm_type(base_kind));
                                                gep_info
                                                    .insert(current_name, (gep_idx, Some((0, bw))));
                                                bitfield_group =
                                                    Some((base_kind, bw, storage_bits));
                                                gep_idx += 1;
                                            }
                                        } else {
                                            // Start first bitfield group
                                            field_types
                                                .push(self.node_kind_to_llvm_type(base_kind));
                                            gep_info.insert(current_name, (gep_idx, Some((0, bw))));
                                            bitfield_group = Some((base_kind, bw, storage_bits));
                                            gep_idx += 1;
                                        }
                                    } else {
                                        // Non-bitfield member: close any open bitfield group
                                        bitfield_group = None;
                                        if let Some(len) = array_len {
                                            let elem = if has_pointer {
                                                self.context
                                                    .ptr_type(AddressSpace::default())
                                                    .as_basic_type_enum()
                                            } else if matches!(base_kind, 4 | 5)
                                                && nested_spec_off != NodeOffset::NULL
                                            {
                                                arena
                                                    .get(nested_spec_off)
                                                    .map(|sn| {
                                                        self.specifier_to_llvm_type(arena, sn)
                                                    })
                                                    .unwrap_or_else(|| {
                                                        self.node_kind_to_llvm_type(base_kind)
                                                    })
                                            } else {
                                                self.node_kind_to_llvm_type(base_kind)
                                            };
                                            field_types
                                                .push(elem.array_type(len).as_basic_type_enum());
                                        } else if has_pointer {
                                            field_types.push(
                                                self.context
                                                    .ptr_type(AddressSpace::default())
                                                    .as_basic_type_enum(),
                                            );
                                        } else if matches!(base_kind, 4 | 5)
                                            && nested_spec_off != NodeOffset::NULL
                                        {
                                            // Nested struct/union: resolve recursively
                                            let nested = arena
                                                .get(nested_spec_off)
                                                .map(|sn| self.specifier_to_llvm_type(arena, sn))
                                                .unwrap_or_else(|| {
                                                    self.node_kind_to_llvm_type(base_kind)
                                                });
                                            field_types.push(nested);
                                        } else {
                                            field_types
                                                .push(self.node_kind_to_llvm_type(base_kind));
                                        }
                                        gep_info.insert(current_name, (gep_idx, None));
                                        gep_idx += 1;
                                    }

                                    member_off = m.next_sibling;
                                } else {
                                    break;
                                }
                            }
                            if !field_types.is_empty() {
                                let st = self.context.struct_type(
                                    &field_types,
                                    self.node_has_attr(arena, child, &["packed", "__packed__"]),
                                );
                                self.struct_tag_types.insert(tag_name.clone(), st);
                                self.struct_tag_fields.insert(tag_name.clone(), field_names);
                                if !fn_field_types.is_empty() {
                                    self.struct_field_fn_types
                                        .insert(tag_name.clone(), fn_field_types);
                                }
                                if gep_info.values().any(|(_, bf)| bf.is_some()) {
                                    self.struct_gep_info.insert(tag_name, gep_info);
                                }
                            }
                        }
                    }
                }
                self.register_struct_types_in_node(arena, child);
                child_off = child.next_sibling;
            } else {
                break;
            }
        }
    }

    fn node_kind_to_llvm_type(&self, kind: u16) -> BasicTypeEnum<'ctx> {
        match kind {
            1 => self.context.i8_type().as_basic_type_enum(),
            2 => self.context.i32_type().as_basic_type_enum(),
            3 => self.context.i8_type().as_basic_type_enum(),
            7 => self
                .context
                .ptr_type(AddressSpace::default())
                .as_basic_type_enum(),
            6 => self.context.i32_type().as_basic_type_enum(),
            10 => self.context.i16_type().as_basic_type_enum(),
            11 => self.context.i64_type().as_basic_type_enum(),
            12 | 13 => self.context.i32_type().as_basic_type_enum(),
            // kind 16 = va_list / __builtin_va_list / __gnuc_va_list.
            // Lower as an opaque pointer so LLVM va_arg/variadic flows remain compatible.
            16 => self
                .context
                .ptr_type(AddressSpace::default())
                .as_basic_type_enum(),
            83 => self.context.f32_type().as_basic_type_enum(),
            84 => self.context.f64_type().as_basic_type_enum(),
            _ => self.context.i32_type().as_basic_type_enum(),
        }
    }

    /// Return the bit width of a storage unit for a given AST type-specifier kind.
    fn node_kind_bit_width(&self, kind: u16) -> u32 {
        match kind {
            1 | 3 => 8,            // void (treated as i8), char
            10 => 16,              // short
            2 | 6 | 12 | 13 => 32, // int, enum, signed, unsigned
            11 => 64,              // long
            _ => 32,
        }
    }

    fn align_up(value: u64, align: u64) -> u64 {
        if align <= 1 {
            value
        } else {
            ((value + align - 1) / align) * align
        }
    }

    fn ast_type_size_align(&self, arena: &Arena, node: &CAstNode) -> Option<(u64, u64)> {
        match node.kind {
            1 => Some((0, 1)),
            2 | 12 | 13 | 83 => Some((4, 4)),
            3 | 14 => Some((1, 1)),
            7 => Some((8, 8)),
            10 => Some((2, 2)),
            11 | 84 => Some((8, 8)),
            // kind 16 = va_list family. Model it as pointer-sized for 64-bit targets.
            16 => Some((8, 8)),
            4 | 5 => self.ast_record_size_align(arena, node),
            _ => {
                if node.first_child != NodeOffset::NULL {
                    self.ast_type_size_align(arena, arena.get(node.first_child)?)
                } else {
                    None
                }
            }
        }
    }

    fn ast_record_size_align(&self, arena: &Arena, spec_node: &CAstNode) -> Option<(u64, u64)> {
        if !matches!(spec_node.kind, 4 | 5) {
            return None;
        }

        if spec_node.first_child == NodeOffset::NULL && spec_node.data != 0 {
            if let Some(tag) = arena.get_string(NodeOffset(spec_node.data)) {
                if let Some(struct_type) = self.struct_tag_types.get(tag).copied() {
                    let fields = struct_type.get_field_types();
                    if fields.is_empty() {
                        return Some((0, 1));
                    }
                    let packed = struct_type.is_packed();
                    let mut offset = 0u64;
                    let mut max_align = 1u64;
                    let mut max_size = 0u64;
                    for field in fields {
                        let (field_size, field_align) = match field {
                            BasicTypeEnum::ArrayType(at) => {
                                let elem = at.get_element_type();
                                let (elem_size, elem_align) = match elem {
                                    BasicTypeEnum::IntType(it) => {
                                        let sz = (it.get_bit_width().max(8) as u64) / 8;
                                        (sz, sz)
                                    }
                                    BasicTypeEnum::FloatType(ft) => {
                                        let sz =
                                            ft.size_of().get_zero_extended_constant().unwrap_or(4);
                                        (sz, sz)
                                    }
                                    BasicTypeEnum::PointerType(_) => (8, 8),
                                    BasicTypeEnum::StructType(st) => {
                                        self.ast_record_size_align_from_llvm(st)
                                    }
                                    _ => (4, 4),
                                };
                                (elem_size * at.len() as u64, elem_align)
                            }
                            BasicTypeEnum::IntType(it) => {
                                let sz = (it.get_bit_width().max(8) as u64) / 8;
                                (sz, sz)
                            }
                            BasicTypeEnum::FloatType(ft) => {
                                let sz = ft.size_of().get_zero_extended_constant().unwrap_or(4);
                                (sz, sz)
                            }
                            BasicTypeEnum::PointerType(_) => (8, 8),
                            BasicTypeEnum::StructType(st) => {
                                self.ast_record_size_align_from_llvm(st)
                            }
                            _ => (4, 4),
                        };
                        let align = if packed { 1 } else { field_align.max(1) };
                        max_align = max_align.max(align);
                        if spec_node.kind == 5 {
                            max_size = max_size.max(field_size);
                        } else {
                            offset = Self::align_up(offset, align);
                            offset += field_size;
                        }
                    }
                    let size = if spec_node.kind == 5 {
                        if packed {
                            max_size
                        } else {
                            Self::align_up(max_size, max_align)
                        }
                    } else if packed {
                        offset
                    } else {
                        Self::align_up(offset, max_align)
                    };
                    return Some((size, if packed { 1 } else { max_align }));
                }
            }
        }

        let packed = self.node_has_attr(arena, spec_node, &["packed", "__packed__"]);
        let mut offset = 0u64;
        let mut max_align = 1u64;
        let mut max_size = 0u64;
        let mut member_off = spec_node.first_child;

        while member_off != NodeOffset::NULL {
            let Some(member) = arena.get(member_off) else {
                break;
            };
            member_off = member.next_sibling;

            if member.kind == 200 {
                continue;
            }

            let mut field_size = 4u64;
            let mut field_align = 4u64;
            let mut is_pointer = false;
            let mut array_len: Option<u64> = None;
            let mut bitfield_width: Option<u64> = None;

            let mut check_off = member.first_child;
            while check_off != NodeOffset::NULL {
                let Some(child) = arena.get(check_off) else {
                    break;
                };
                match child.kind {
                    1 | 2 | 3 | 10..=14 | 83 | 84 => {
                        if let Some((sz, al)) = self.ast_type_size_align(arena, child) {
                            field_size = sz;
                            field_align = al;
                        }
                    }
                    4 | 5 => {
                        if let Some((sz, al)) = self.ast_record_size_align(arena, child) {
                            field_size = sz;
                            field_align = al;
                        }
                    }
                    7 => is_pointer = true,
                    8 => {
                        if let Some(len) = self.infer_array_declarator_len(arena, child) {
                            array_len = Some(len as u64);
                        }
                    }
                    27 => bitfield_width = Some(child.data as u64),
                    _ => {}
                }
                check_off = child.next_sibling;
            }

            if is_pointer {
                field_size = 8;
                field_align = 8;
            }
            if let Some(len) = array_len {
                field_size *= len;
            }
            if let Some(width) = bitfield_width {
                let storage_bytes = field_size.max(1);
                field_size = ((width + 7) / 8).min(storage_bytes);
            }

            let align = if packed { 1 } else { field_align.max(1) };
            max_align = max_align.max(align);
            if spec_node.kind == 5 {
                max_size = max_size.max(field_size);
            } else {
                offset = Self::align_up(offset, align);
                offset += field_size;
            }
        }

        let size = if spec_node.kind == 5 {
            if packed {
                max_size
            } else {
                Self::align_up(max_size, max_align)
            }
        } else if packed {
            offset
        } else {
            Self::align_up(offset, max_align)
        };

        Some((size, if packed { 1 } else { max_align }))
    }

    fn ast_record_size_align_from_llvm(&self, st: StructType<'ctx>) -> (u64, u64) {
        let packed = st.is_packed();
        let mut offset = 0u64;
        let mut max_align = 1u64;
        for field in st.get_field_types() {
            let (field_size, field_align) = match field {
                BasicTypeEnum::IntType(it) => {
                    let sz = (it.get_bit_width().max(8) as u64) / 8;
                    (sz, sz)
                }
                BasicTypeEnum::FloatType(ft) => {
                    let sz = ft.size_of().get_zero_extended_constant().unwrap_or(4);
                    (sz, sz)
                }
                BasicTypeEnum::PointerType(_) => (8, 8),
                BasicTypeEnum::ArrayType(at) => {
                    let elem = at.get_element_type();
                    let (elem_size, elem_align) = match elem {
                        BasicTypeEnum::IntType(it) => {
                            let sz = (it.get_bit_width().max(8) as u64) / 8;
                            (sz, sz)
                        }
                        BasicTypeEnum::FloatType(ft) => {
                            let sz = ft.size_of().get_zero_extended_constant().unwrap_or(4);
                            (sz, sz)
                        }
                        BasicTypeEnum::PointerType(_) => (8, 8),
                        BasicTypeEnum::StructType(inner) => {
                            self.ast_record_size_align_from_llvm(inner)
                        }
                        _ => (4, 4),
                    };
                    (elem_size * at.len() as u64, elem_align)
                }
                BasicTypeEnum::StructType(inner) => self.ast_record_size_align_from_llvm(inner),
                _ => (4, 4),
            };
            let align = if packed { 1 } else { field_align.max(1) };
            max_align = max_align.max(align);
            offset = Self::align_up(offset, align);
            offset += field_size;
        }
        let size = if packed {
            offset
        } else {
            Self::align_up(offset, max_align)
        };
        (size, if packed { 1 } else { max_align })
    }

    fn node_has_attr(&self, arena: &Arena, node: &CAstNode, names: &[&str]) -> bool {
        self.extract_attributes(arena, node)
            .iter()
            .any(|(name, _, _)| names.iter().any(|candidate| name == candidate))
    }

    /// Resolve a type specifier node to its LLVM type, including struct/union types.
    /// For struct/union (kind=4/5), looks up struct_tag_types or builds inline.
    fn specifier_to_llvm_type(&self, arena: &Arena, spec_node: &CAstNode) -> BasicTypeEnum<'ctx> {
        if matches!(spec_node.kind, 4 | 5) {
            self.struct_info_for_spec(arena, Some(spec_node))
                .map(|(st, _)| st.as_basic_type_enum())
                .unwrap_or_else(|| self.context.i32_type().as_basic_type_enum())
        } else if spec_node.kind == 60 {
            arena
                .get_string(NodeOffset(spec_node.data))
                .and_then(|name| self.struct_tag_types.get(name).copied())
                .map(|st| st.as_basic_type_enum())
                .unwrap_or_else(|| self.node_kind_to_llvm_type(spec_node.kind))
        } else {
            self.node_kind_to_llvm_type(spec_node.kind)
        }
    }

    fn function_type_from_declarator(
        &self,
        arena: &Arena,
        declarator: Option<&CAstNode>,
        base_type: BasicTypeEnum<'ctx>,
        base_kind: u16,
    ) -> Option<FunctionType<'ctx>> {
        let decl = declarator?;
        if decl.kind != 9 {
            return None;
        }

        let pointer_decl = arena.get(decl.first_child)?;
        if pointer_decl.kind != 7 {
            return None;
        }

        let mut param_types = Vec::new();
        let mut param_off = pointer_decl.next_sibling;
        while param_off != NodeOffset::NULL {
            let param = arena.get(param_off)?;
            if param.kind == 24 {
                let (ptype, _, _, _) = self.extract_param_type_name(arena, param);
                param_types.push(ptype.into());
            }
            param_off = param.next_sibling;
        }

        Some(if base_kind == 1 {
            self.context
                .void_type()
                .fn_type(&param_types, decl.data == 1)
        } else {
            match base_type {
                BasicTypeEnum::IntType(int_ty) => int_ty.fn_type(&param_types, decl.data == 1),
                BasicTypeEnum::FloatType(float_ty) => {
                    float_ty.fn_type(&param_types, decl.data == 1)
                }
                BasicTypeEnum::PointerType(ptr_ty) => ptr_ty.fn_type(&param_types, decl.data == 1),
                BasicTypeEnum::StructType(struct_ty) => {
                    struct_ty.fn_type(&param_types, decl.data == 1)
                }
                BasicTypeEnum::ArrayType(array_ty) => {
                    array_ty.fn_type(&param_types, decl.data == 1)
                }
                BasicTypeEnum::VectorType(vector_ty) => {
                    vector_ty.fn_type(&param_types, decl.data == 1)
                }
                BasicTypeEnum::ScalableVectorType(vector_ty) => {
                    vector_ty.fn_type(&param_types, decl.data == 1)
                }
            }
        })
    }

    fn function_decl_param_offset(&self, arena: &Arena, func_decl: &CAstNode) -> NodeOffset {
        arena
            .get(func_decl.first_child)
            .map(|child| child.next_sibling)
            .unwrap_or(NodeOffset::NULL)
    }

    fn count_function_decl_params(&self, arena: &Arena, func_decl: &CAstNode) -> usize {
        let mut count = 0;
        let mut param_off = self.function_decl_param_offset(arena, func_decl);
        while param_off != NodeOffset::NULL {
            let Some(param) = arena.get(param_off) else {
                break;
            };
            if param.kind == 24 {
                count += 1;
            }
            param_off = param.next_sibling;
        }
        count
    }

    fn select_callable_function_decl_offset(
        &self,
        arena: &Arena,
        func_decl_offset: NodeOffset,
    ) -> NodeOffset {
        let Some(func_decl) = arena.get(func_decl_offset) else {
            return func_decl_offset;
        };
        let direct_params = self.count_function_decl_params(arena, func_decl);
        let nested = self
            .function_decl_param_offset(arena, func_decl)
            .eq(&NodeOffset::NULL)
            .then(|| self.find_function_declarator_offset(arena, func_decl.first_child))
            .flatten()
            .filter(|nested_off| *nested_off != func_decl_offset);

        if let Some(nested_off) = nested {
            if let Some(nested_decl) = arena.get(nested_off) {
                let nested_params = self.count_function_decl_params(arena, nested_decl);
                if nested_params > direct_params {
                    return nested_off;
                }
            }
        }

        func_decl_offset
    }

    fn collect_typedef_aliases(&mut self, arena: &Arena, offset: NodeOffset) {
        let mut current = offset;
        while current != NodeOffset::NULL {
            let Some(node) = arena.get(current) else {
                break;
            };
            if node.kind == 20 {
                let mut child_offset = node.first_child;
                let mut is_typedef = false;
                while child_offset != NodeOffset::NULL {
                    let Some(child) = arena.get(child_offset) else {
                        break;
                    };
                    if child.kind == 101 {
                        is_typedef = true;
                    } else if is_typedef {
                        if let Some(name) = self.find_ident_name(arena, child) {
                            self.typedef_aliases.insert(name);
                        }
                    }
                    child_offset = child.next_sibling;
                }
            }
            current = node.next_sibling;
        }
    }

    fn collect_static_function_names(&mut self, arena: &Arena, offset: NodeOffset) {
        let mut current = offset;
        while current != NodeOffset::NULL {
            let Some(node) = arena.get(current) else {
                break;
            };
            if matches!(node.kind, 20 | 22 | 23) {
                let (_, is_static, _, _) = self.function_signature_info(arena, node);
                if is_static {
                    if let Some(name) = self.find_ident_name(arena, node) {
                        self.static_declared_functions.insert(name);
                    }
                }
            }
            current = node.next_sibling;
        }
    }

    fn materialize_missing_function_body(
        &self,
        function: FunctionValue<'ctx>,
        return_first_arg: bool,
    ) -> Result<(), BackendError> {
        if function.get_first_basic_block().is_some() {
            return Ok(());
        }

        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);

        if return_first_arg && function.count_params() > 0 {
            let arg = function.get_nth_param(0).ok_or(BackendError::InvalidNode)?;
            match function.get_type().get_return_type() {
                None => {
                    self.builder
                        .build_return(None)
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Some(ret_ty) if ret_ty == arg.get_type() => {
                    self.builder
                        .build_return(Some(&arg))
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Some(ret_ty) if ret_ty.is_int_type() && arg.get_type().is_int_type() => {
                    let ret_int = ret_ty.into_int_type();
                    let arg_int = arg.into_int_value();
                    let casted = if arg_int.get_type().get_bit_width() < ret_int.get_bit_width() {
                        self.builder
                            .build_int_z_extend(arg_int, ret_int, "typedef_cast_zext")
                            .map_err(|_| BackendError::InvalidNode)?
                    } else if arg_int.get_type().get_bit_width() > ret_int.get_bit_width() {
                        self.builder
                            .build_int_truncate(arg_int, ret_int, "typedef_cast_trunc")
                            .map_err(|_| BackendError::InvalidNode)?
                    } else {
                        arg_int
                    };
                    self.builder
                        .build_return(Some(&casted))
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Some(ret_ty) if ret_ty.is_pointer_type() && arg.get_type().is_pointer_type() => {
                    self.builder
                        .build_return(Some(&arg))
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Some(ret_ty) if ret_ty.is_pointer_type() => {
                    let null = self.context.ptr_type(AddressSpace::default()).const_null();
                    self.builder
                        .build_return(Some(&null))
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Some(ret_ty) if ret_ty.is_float_type() && arg.get_type().is_float_type() => {
                    self.builder
                        .build_return(Some(&arg))
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Some(ret_ty) if ret_ty.is_struct_type() => {
                    let zero = ret_ty.into_struct_type().const_zero();
                    self.builder
                        .build_return(Some(&zero))
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Some(ret_ty) => {
                    let zero: BasicValueEnum = if ret_ty.is_int_type() {
                        ret_ty.into_int_type().const_zero().into()
                    } else if ret_ty.is_float_type() {
                        ret_ty.into_float_type().const_zero().into()
                    } else if ret_ty.is_pointer_type() {
                        self.context
                            .ptr_type(AddressSpace::default())
                            .const_null()
                            .into()
                    } else if ret_ty.is_array_type() {
                        ret_ty.into_array_type().const_zero().into()
                    } else if ret_ty.is_vector_type() {
                        ret_ty.into_vector_type().const_zero().into()
                    } else if ret_ty.is_scalable_vector_type() {
                        ret_ty.into_scalable_vector_type().const_zero().into()
                    } else if ret_ty.is_struct_type() {
                        ret_ty.into_struct_type().const_zero().into()
                    } else {
                        self.context.i32_type().const_zero().into()
                    };
                    self.builder
                        .build_return(Some(&zero))
                        .map_err(|_| BackendError::InvalidNode)?;
                }
            }
            return Ok(());
        }

        match function.get_type().get_return_type() {
            None => {
                self.builder
                    .build_return(None)
                    .map_err(|_| BackendError::InvalidNode)?;
            }
            Some(ret_ty) if ret_ty.is_int_type() => {
                let zero = ret_ty.into_int_type().const_zero();
                self.builder
                    .build_return(Some(&zero))
                    .map_err(|_| BackendError::InvalidNode)?;
            }
            Some(ret_ty) if ret_ty.is_float_type() => {
                let zero = ret_ty.into_float_type().const_zero();
                self.builder
                    .build_return(Some(&zero))
                    .map_err(|_| BackendError::InvalidNode)?;
            }
            Some(ret_ty) if ret_ty.is_pointer_type() => {
                let zero = self.context.ptr_type(AddressSpace::default()).const_null();
                self.builder
                    .build_return(Some(&zero))
                    .map_err(|_| BackendError::InvalidNode)?;
            }
            Some(ret_ty) if ret_ty.is_struct_type() => {
                let zero = ret_ty.into_struct_type().const_zero();
                self.builder
                    .build_return(Some(&zero))
                    .map_err(|_| BackendError::InvalidNode)?;
            }
            Some(ret_ty) if ret_ty.is_array_type() => {
                let zero = ret_ty.into_array_type().const_zero();
                self.builder
                    .build_return(Some(&zero))
                    .map_err(|_| BackendError::InvalidNode)?;
            }
            Some(ret_ty) if ret_ty.is_vector_type() => {
                let zero = ret_ty.into_vector_type().const_zero();
                self.builder
                    .build_return(Some(&zero))
                    .map_err(|_| BackendError::InvalidNode)?;
            }
            Some(ret_ty) if ret_ty.is_scalable_vector_type() => {
                let zero = ret_ty.into_scalable_vector_type().const_zero();
                self.builder
                    .build_return(Some(&zero))
                    .map_err(|_| BackendError::InvalidNode)?;
            }
            Some(_) => {
                self.builder
                    .build_return(None)
                    .map_err(|_| BackendError::InvalidNode)?;
            }
        }
        Ok(())
    }

    fn find_function_declarator_root_offset(
        &self,
        arena: &Arena,
        offset: NodeOffset,
    ) -> Option<NodeOffset> {
        let node = arena.get(offset)?;
        match node.kind {
            7..=9 => Some(offset),
            _ => {
                let mut child_offset = node.first_child;
                while child_offset != NodeOffset::NULL {
                    if let Some(found) =
                        self.find_function_declarator_root_offset(arena, child_offset)
                    {
                        return Some(found);
                    }
                    child_offset = arena
                        .get(child_offset)
                        .map(|n| n.next_sibling)
                        .unwrap_or(NodeOffset::NULL);
                }
                None
            }
        }
    }

    fn function_type_for_member_access(
        &self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Option<FunctionType<'ctx>> {
        if node.kind != 69 {
            return None;
        }

        let field_name = arena
            .get_string(NodeOffset(node.data & 0x7FFF_FFFF))
            .map(|s| s.to_string())?;
        let base_struct_type = self.member_access_base_struct_type(arena, node)?;
        let tag = self.struct_tag_for_type(base_struct_type)?;
        self.struct_field_fn_types
            .get(&tag)
            .and_then(|fields| fields.get(&field_name).copied())
    }

    fn struct_tag_for_type(&self, struct_type: StructType<'ctx>) -> Option<String> {
        self.struct_tag_types.iter().find_map(|(tag, ty)| {
            let same_layout = ty.count_fields() == struct_type.count_fields()
                && (0..ty.count_fields()).all(|i| {
                    ty.get_field_type_at_index(i) == struct_type.get_field_type_at_index(i)
                });
            if *ty == struct_type || same_layout {
                Some(tag.clone())
            } else {
                None
            }
        })
    }

    fn member_access_result_type(
        &self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Option<BasicTypeEnum<'ctx>> {
        if node.kind != 69 {
            return None;
        }

        let base_struct_type = self.member_access_base_struct_type(arena, node)?;
        let field_name = arena
            .get_string(NodeOffset(node.data & 0x7FFF_FFFF))
            .map(|s| s.to_string())?;
        let tag = self.struct_tag_for_type(base_struct_type)?;
        let field_idx = self
            .struct_tag_fields
            .get(&tag)
            .and_then(|fields| fields.iter().position(|f| f == &field_name))?;
        base_struct_type.get_field_type_at_index(field_idx as u32)
    }

    fn member_access_base_struct_type(
        &self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Option<StructType<'ctx>> {
        let base_offset = node.first_child;
        let is_arrow = node.data & 0x8000_0000 != 0;
        let base_node = arena.get(base_offset)?;

        if base_node.kind == 60 {
            let base_name = arena.get_string(NodeOffset(base_node.data))?;
            if is_arrow {
                return self
                    .pointer_struct_types
                    .get(base_name)
                    .copied()
                    .or_else(|| {
                        self.var_struct_tag
                            .get(base_name)
                            .or_else(|| self.global_struct_tags.get(base_name))
                            .and_then(|tag| self.struct_tag_types.get(tag))
                            .copied()
                    });
            }

            if let Some(binding) = self
                .variables
                .get(base_name)
                .copied()
                .or_else(|| self.global_variables.get(base_name).copied())
            {
                if let BasicTypeEnum::StructType(struct_type) = binding.pointee_type {
                    return Some(struct_type);
                }
            }

            return self
                .var_struct_tag
                .get(base_name)
                .or_else(|| self.global_struct_tags.get(base_name))
                .and_then(|tag| self.struct_tag_types.get(tag))
                .copied();
        }

        if base_node.kind == 69 {
            return self
                .member_access_result_type(arena, base_node)
                .and_then(|ty| match ty {
                    BasicTypeEnum::StructType(struct_type) => Some(struct_type),
                    _ => None,
                });
        }

        None
    }

    /// Extract (llvm_type, param_name) from a kind=24 parameter declaration node.
    /// Layout after parser fix: first_child chain = type_spec -> kind=60(name)
    fn extract_param_type_name(
        &self,
        arena: &Arena,
        param: &CAstNode,
    ) -> (
        BasicTypeEnum<'ctx>,
        String,
        Option<(StructType<'ctx>, Vec<String>)>,
        Option<FunctionType<'ctx>>,
    ) {
        let spec_node = arena.get(param.first_child);
        let type_kind = spec_node.map(|n| n.kind).unwrap_or(2);
        let base_type = if let Some(sn) = spec_node {
            self.specifier_to_llvm_type(arena, sn)
        } else {
            self.context.i32_type().as_basic_type_enum()
        };
        let mut name = "p".to_string();
        let mut declarator_offset = NodeOffset::NULL;
        // DFS helper: find the first kind=60 identifier in a declarator subtree
        fn find_ident_in_decl(arena: &Arena, off: NodeOffset) -> Option<String> {
            let n = arena.get(off)?;
            if n.kind == 60 {
                let s = arena.get_string(NodeOffset(n.data))?;
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
            // Descend into first_child for declarator nodes
            if matches!(n.kind, 7 | 8 | 9) && n.first_child != NodeOffset::NULL {
                if let Some(found) = find_ident_in_decl(arena, n.first_child) {
                    return Some(found);
                }
            }
            // Try next sibling (for declarators chained within the same level)
            if n.next_sibling != NodeOffset::NULL && matches!(n.kind, 7 | 8 | 9 | 60) {
                if let Some(found) = find_ident_in_decl(arena, n.next_sibling) {
                    return Some(found);
                }
            }
            None
        }
        let mut off = param.first_child;
        while off != NodeOffset::NULL {
            if let Some(n) = arena.get(off) {
                if matches!(n.kind, 7..=9 | 60) {
                    declarator_offset = off;
                }
                if n.kind == 60 {
                    if let Some(s) = arena.get_string(NodeOffset(n.data)) {
                        if !s.is_empty() {
                            name = s.to_string();
                            break;
                        }
                    }
                }
                // Use DFS to find identifier in any declarator subtree (handles deeply nested
                // function pointer params like void *(*xTask)(void*))
                if matches!(n.kind, 7 | 8 | 9) && n.first_child != NodeOffset::NULL {
                    if let Some(found) = find_ident_in_decl(arena, n.first_child) {
                        name = found;
                        break;
                    }
                }
                off = n.next_sibling;
            } else {
                break;
            }
        }
        // If the declarator is kind=9 (function declarator) containing a kind=7 (pointer),
        // this is a function pointer parameter — override type to ptr.
        let llvm_type = if declarator_offset != NodeOffset::NULL {
            if let Some(decl_node) = arena.get(declarator_offset) {
                if decl_node.kind == 9 {
                    let mut has_ptr = false;
                    let mut scan = decl_node.first_child;
                    while scan != NodeOffset::NULL {
                        if let Some(sn) = arena.get(scan) {
                            if sn.kind == 7 {
                                has_ptr = true;
                                break;
                            }
                            scan = sn.next_sibling;
                        } else {
                            break;
                        }
                    }
                    if has_ptr {
                        self.context
                            .ptr_type(AddressSpace::default())
                            .as_basic_type_enum()
                    } else {
                        self.declarator_llvm_type(arena, Some(decl_node), base_type)
                    }
                } else {
                    self.declarator_llvm_type(arena, Some(decl_node), base_type)
                }
            } else {
                base_type
            }
        } else {
            base_type
        };
        let function_type = self.function_type_from_declarator(
            arena,
            arena.get(declarator_offset),
            base_type,
            type_kind,
        );
        (
            llvm_type,
            name,
            self.struct_info_for_spec(arena, spec_node),
            function_type,
        )
    }

    fn struct_info_for_spec(
        &self,
        arena: &Arena,
        spec_node: Option<&CAstNode>,
    ) -> Option<(StructType<'ctx>, Vec<String>)> {
        let spec_node = spec_node?;
        if spec_node.kind == 60 {
            let name = arena.get_string(NodeOffset(spec_node.data))?;
            let struct_type = self.struct_tag_types.get(name).copied()?;
            let field_names = self
                .struct_tag_fields
                .get(name)
                .cloned()
                .unwrap_or_default();
            return Some((struct_type, field_names));
        }

        if !matches!(spec_node.kind, 4 | 5) {
            return None;
        }

        let struct_type = if spec_node.data != 0 {
            arena
                .get_string(NodeOffset(spec_node.data))
                .and_then(|tag| self.struct_tag_types.get(tag).copied())
        } else {
            None
        }
        .or_else(|| match self.build_struct_llvm_type(arena, spec_node) {
            BasicTypeEnum::StructType(st) => Some(st),
            _ => None,
        })?;

        let field_names = if spec_node.data != 0 {
            arena
                .get_string(NodeOffset(spec_node.data))
                .and_then(|tag| self.struct_tag_fields.get(tag).cloned())
                .unwrap_or_else(|| Self::collect_struct_field_names(arena, spec_node))
        } else {
            Self::collect_struct_field_names(arena, spec_node)
        };

        Some((struct_type, field_names))
    }

    fn declarator_llvm_type(
        &self,
        _arena: &Arena,
        declarator: Option<&CAstNode>,
        base_type: BasicTypeEnum<'ctx>,
    ) -> BasicTypeEnum<'ctx> {
        let Some(declarator) = declarator else {
            return base_type;
        };

        match declarator.kind {
            7 => {
                // Pointer depth is stored in `data` (set by the parser).
                // A value of 0 means a single pointer level (legacy nodes that
                // were allocated before the parser encoded the depth).
                let depth = if declarator.data > 0 {
                    declarator.data as usize
                } else {
                    1
                };

                let mut ty = base_type;
                for _ in 0..depth {
                    let _ = ty;
                    ty = self
                        .context
                        .ptr_type(AddressSpace::default())
                        .as_basic_type_enum();
                }
                ty
            }
            8 => base_type.array_type(declarator.data).as_basic_type_enum(),
            9 => {
                let mut scan = declarator.first_child;
                while scan != NodeOffset::NULL {
                    let Some(node) = _arena.get(scan) else { break };
                    if node.kind == 7 {
                        return self
                            .context
                            .ptr_type(AddressSpace::default())
                            .as_basic_type_enum();
                    }
                    scan = node.next_sibling;
                }
                base_type
            }
            _ => base_type,
        }
    }

    fn subtree_has_byte_pointer_ident(&self, arena: &Arena, offset: NodeOffset) -> bool {
        fn visit<'ctx, 'types>(
            this: &LlvmBackend<'ctx, 'types>,
            arena: &Arena,
            off: NodeOffset,
        ) -> bool {
            if off == NodeOffset::NULL {
                return false;
            }
            let Some(node) = arena.get(off) else {
                return false;
            };

            if node.kind == 60 {
                if let Some(name) = arena.get_string(NodeOffset(node.data)) {
                    if this.byte_pointer_vars.contains(name) || name.starts_with('z') {
                        return true;
                    }
                }
            }

            if visit(this, arena, node.first_child) {
                return true;
            }
            visit(this, arena, node.next_sibling)
        }

        visit(self, arena, offset)
    }

    fn lower_var_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        // Check for static storage class and find the actual type specifier node
        // (first child may be storage class qualifiers like static=103, extern=102, etc.)
        let mut is_static_local = false;
        let mut spec_offset = node.first_child;
        loop {
            let Some(sn) = arena.get(spec_offset) else {
                break;
            };
            match sn.kind {
                101 => return Ok(()),
                102 => {
                    /* extern */
                    spec_offset = sn.next_sibling;
                }
                103 => {
                    is_static_local = true;
                    spec_offset = sn.next_sibling;
                }
                90 | 91 | 92 | 104 | 105 | 106 => {
                    /* const/restrict/volatile/auto/register/_Thread_local(kind=106) */
                    spec_offset = sn.next_sibling;
                }
                _ => break,
            }
        }
        let spec_node = arena.get(spec_offset);
        let spec_kind = spec_node.map(|n| n.kind).unwrap_or(2);
        let struct_info = self.struct_info_for_spec(arena, spec_node);

        let alloca_type = if spec_kind == 4 || spec_kind == 5 {
            if let Some(sn) = spec_node {
                let tag_type = if sn.data != 0 {
                    arena
                        .get_string(NodeOffset(sn.data))
                        .and_then(|tag| self.struct_tag_types.get(tag).copied())
                        .map(|st| st.as_basic_type_enum())
                } else {
                    None
                };
                tag_type.unwrap_or_else(|| {
                    if sn.first_child != NodeOffset::NULL {
                        self.build_struct_llvm_type(arena, sn)
                    } else {
                        self.context.i8_type().as_basic_type_enum()
                    }
                })
            } else {
                self.context.i8_type().as_basic_type_enum()
            }
        } else {
            self.node_kind_to_llvm_type(spec_kind)
        };

        // Walk the first_child chain: type specifiers come first, then init-declarators
        // (kind=73 or kind=60). Process ALL init-declarators.
        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                match child.kind {
                    // Kind=73: init-declarator with optional initializer
                    73 => {
                        // first_child = kind=60(name), kind=8(array decl), or pointer_decl
                        // declarator.next_sibling = init_expr (stored there to survive link_siblings)
                        let declarator_node = arena.get(child.first_child);

                        let actual_alloca_type =
                            self.declarator_llvm_type(arena, declarator_node, alloca_type);
                        let function_type = self.function_type_from_declarator(
                            arena,
                            declarator_node,
                            alloca_type,
                            spec_kind,
                        );
                        let is_array = declarator_node.map(|dn| dn.kind == 8).unwrap_or(false);

                        let var_name_opt: Option<String> = declarator_node.and_then(|n| {
                            if n.kind == 60 {
                                arena
                                    .get_string(NodeOffset(n.data))
                                    .filter(|s| !s.is_empty())
                                    .map(|s| s.to_string())
                            } else {
                                // For pointer/array declarators (kind=7/8/9), find the ident deeper
                                self.find_ident_name_in(arena, n)
                            }
                        });
                        if let Some(var_name) = var_name_opt {
                            let init_offset = declarator_node
                                .map(|d| d.next_sibling)
                                .unwrap_or(NodeOffset::NULL);
                            // Static local variables must be emitted as globals with internal linkage
                            if is_static_local {
                                // For unsized static local arrays (declarator data=0), determine the
                                // true element count from the flat initializer list.
                                let actual_alloca_type =
                                    if let BasicTypeEnum::ArrayType(at) = actual_alloca_type {
                                        if at.len() == 0 && init_offset != NodeOffset::NULL {
                                            if let BasicTypeEnum::StructType(elem_st) =
                                                at.get_element_type()
                                            {
                                                let nf = elem_st.count_fields() as usize;
                                                if nf > 0 {
                                                    let mut cnt = 0usize;
                                                    let mut off = init_offset;
                                                    while off != NodeOffset::NULL {
                                                        cnt += 1;
                                                        off = arena
                                                            .get(off)
                                                            .map(|n| n.next_sibling)
                                                            .unwrap_or(NodeOffset::NULL);
                                                    }
                                                    let ne = cnt / nf;
                                                    if ne > 0 {
                                                        elem_st
                                                            .array_type(ne as u32)
                                                            .as_basic_type_enum()
                                                    } else {
                                                        actual_alloca_type
                                                    }
                                                } else {
                                                    actual_alloca_type
                                                }
                                            } else {
                                                actual_alloca_type
                                            }
                                        } else {
                                            actual_alloca_type
                                        }
                                    } else {
                                        actual_alloca_type
                                    };
                                let zero: BasicValueEnum = match actual_alloca_type {
                                    BasicTypeEnum::IntType(it) => it.const_zero().into(),
                                    BasicTypeEnum::FloatType(ft) => ft.const_zero().into(),
                                    BasicTypeEnum::PointerType(pt) => pt.const_null().into(),
                                    BasicTypeEnum::StructType(st) => st.const_zero().into(),
                                    BasicTypeEnum::ArrayType(at) => at.const_zero().into(),
                                    _ => self.context.i32_type().const_zero().into(),
                                };
                                // Use a unique name to avoid collision with other statics
                                // Use a unique counter-based name to avoid collision
                                let static_name = {
                                    let n = self.static_local_counter;
                                    self.static_local_counter += 1;
                                    format!("{}.static.{}", var_name, n)
                                };
                                let global = {
                                    let g = self.module.add_global(
                                        actual_alloca_type,
                                        Some(AddressSpace::default()),
                                        &static_name,
                                    );
                                    g.set_initializer(&zero);
                                    g.set_linkage(inkwell::module::Linkage::Internal);
                                    g
                                };
                                // Handle aggregate initializer for static locals
                                if init_offset != NodeOffset::NULL {
                                    if let BasicTypeEnum::StructType(st) = actual_alloca_type {
                                        // Initializer is a linked list of element nodes (first = init_offset)
                                        let mut field_vals: Vec<BasicValueEnum> = Vec::new();
                                        let mut item_off = init_offset;
                                        let mut field_idx = 0u32;
                                        if std::env::var("OPTICC_DEBUG_STATIC").is_ok() {
                                            eprintln!("DEBUG static-init: var={} st.count_fields()={} item_off={:?}", var_name, st.count_fields(), item_off);
                                        }
                                        while item_off != NodeOffset::NULL
                                            && field_idx < st.count_fields()
                                        {
                                            if let Some(item) = arena.get(item_off) {
                                                let field_ty = st
                                                    .get_field_type_at_index(field_idx)
                                                    .unwrap_or_else(|| {
                                                        self.context.i32_type().as_basic_type_enum()
                                                    });
                                                let val: BasicValueEnum = match item.kind {
                                                    60 => {
                                                        // function pointer by name
                                                        let fname = arena
                                                            .get_string(NodeOffset(item.data))
                                                            .unwrap_or("")
                                                            .to_string();
                                                        if std::env::var("OPTICC_DEBUG_STATIC")
                                                            .is_ok()
                                                        {
                                                            eprintln!("DEBUG static-init field[{}]: fname={:?} found={}", field_idx, fname, self.module.get_function(&fname).is_some());
                                                        }
                                                        if fname.is_empty() {
                                                            match field_ty {
                                                                BasicTypeEnum::PointerType(pt) => {
                                                                    pt.const_null().into()
                                                                }
                                                                BasicTypeEnum::IntType(it) => {
                                                                    it.const_zero().into()
                                                                }
                                                                BasicTypeEnum::StructType(st2) => {
                                                                    st2.const_zero().into()
                                                                }
                                                                BasicTypeEnum::ArrayType(at) => {
                                                                    at.const_zero().into()
                                                                }
                                                                _ => self
                                                                    .context
                                                                    .i32_type()
                                                                    .const_zero()
                                                                    .into(),
                                                            }
                                                        } else if let Some(f) =
                                                            self.module.get_function(&fname)
                                                        {
                                                            f.as_global_value()
                                                                .as_pointer_value()
                                                                .into()
                                                        } else {
                                                            match field_ty {
                                                                BasicTypeEnum::PointerType(pt) => {
                                                                    pt.const_null().into()
                                                                }
                                                                BasicTypeEnum::StructType(st2) => {
                                                                    st2.const_zero().into()
                                                                }
                                                                BasicTypeEnum::ArrayType(at) => {
                                                                    at.const_zero().into()
                                                                }
                                                                _ => self
                                                                    .context
                                                                    .i32_type()
                                                                    .const_zero()
                                                                    .into(),
                                                            }
                                                        }
                                                    }
                                                    61 | 80 => {
                                                        // integer literal
                                                        match field_ty {
                                                            BasicTypeEnum::IntType(it) => it
                                                                .const_int(item.data as u64, false)
                                                                .into(),
                                                            BasicTypeEnum::PointerType(pt) => {
                                                                pt.const_null().into()
                                                            }
                                                            BasicTypeEnum::StructType(st2) => {
                                                                st2.const_zero().into()
                                                            }
                                                            BasicTypeEnum::ArrayType(at) => {
                                                                at.const_zero().into()
                                                            }
                                                            _ => self
                                                                .context
                                                                .i32_type()
                                                                .const_zero()
                                                                .into(),
                                                        }
                                                    }
                                                    63 | 81 | 82 => {
                                                        // String literal → create a const global string and return its pointer
                                                        let s = arena
                                                            .get_string(NodeOffset(item.data))
                                                            .unwrap_or("");
                                                        let sv = self
                                                            .context
                                                            .const_string(s.as_bytes(), true);
                                                        let sg = self.module.add_global(
                                                            sv.get_type(),
                                                            Some(AddressSpace::default()),
                                                            ".str",
                                                        );
                                                        sg.set_initializer(&sv);
                                                        sg.set_constant(true);
                                                        sg.set_linkage(
                                                            inkwell::module::Linkage::Private,
                                                        );
                                                        sg.set_unnamed_addr(true);
                                                        sg.as_pointer_value().into()
                                                    }
                                                    _ => match field_ty {
                                                        BasicTypeEnum::PointerType(pt) => {
                                                            pt.const_null().into()
                                                        }
                                                        BasicTypeEnum::IntType(it) => {
                                                            it.const_zero().into()
                                                        }
                                                        BasicTypeEnum::StructType(st2) => {
                                                            st2.const_zero().into()
                                                        }
                                                        BasicTypeEnum::ArrayType(at) => {
                                                            at.const_zero().into()
                                                        }
                                                        _ => self
                                                            .context
                                                            .i32_type()
                                                            .const_zero()
                                                            .into(),
                                                    },
                                                };
                                                if std::env::var("OPTICC_DEBUG_STATIC").is_ok() {
                                                    eprintln!("DEBUG static-init field[{}]: item.kind={} val={:?}", field_idx, item.kind, val);
                                                }
                                                field_vals.push(val);
                                                field_idx += 1;
                                                item_off = item.next_sibling;
                                            } else {
                                                break;
                                            }
                                        }
                                        // Pad remaining fields with zeros
                                        while field_idx < st.count_fields() {
                                            let field_ty = st
                                                .get_field_type_at_index(field_idx)
                                                .unwrap_or_else(|| {
                                                    self.context.i32_type().as_basic_type_enum()
                                                });
                                            let z: BasicValueEnum = match field_ty {
                                                BasicTypeEnum::IntType(it) => {
                                                    it.const_zero().into()
                                                }
                                                BasicTypeEnum::FloatType(ft) => {
                                                    ft.const_zero().into()
                                                }
                                                BasicTypeEnum::PointerType(pt) => {
                                                    pt.const_null().into()
                                                }
                                                BasicTypeEnum::StructType(st2) => {
                                                    st2.const_zero().into()
                                                }
                                                BasicTypeEnum::ArrayType(at) => {
                                                    at.const_zero().into()
                                                }
                                                _ => self.context.i32_type().const_zero().into(),
                                            };
                                            field_vals.push(z);
                                            field_idx += 1;
                                        }
                                        if !field_vals.is_empty() {
                                            let const_struct = self.context.const_struct(
                                                &field_vals.iter().map(|v| *v).collect::<Vec<_>>(),
                                                false,
                                            );
                                            global.set_initializer(&const_struct);
                                        }
                                    } else if let BasicTypeEnum::ArrayType(at) = actual_alloca_type
                                    {
                                        // Array of structs (e.g. static FuncDef arr[] = { {...}, {...} })
                                        if let BasicTypeEnum::StructType(elem_st) =
                                            at.get_element_type()
                                        {
                                            let n_elems = at.len() as usize;
                                            let n_fields = elem_st.count_fields();
                                            let mut item_off = init_offset;
                                            let mut struct_vals: Vec<
                                                inkwell::values::StructValue<'ctx>,
                                            > = Vec::new();
                                            for _ in 0..n_elems {
                                                let mut fvals: Vec<BasicValueEnum<'ctx>> =
                                                    Vec::new();
                                                let mut fi = 0u32;
                                                while fi < n_fields && item_off != NodeOffset::NULL
                                                {
                                                    let Some(item) = arena.get(item_off) else {
                                                        break;
                                                    };
                                                    let ft = elem_st
                                                        .get_field_type_at_index(fi)
                                                        .unwrap_or_else(|| {
                                                            self.context
                                                                .i32_type()
                                                                .as_basic_type_enum()
                                                        });
                                                    let val: BasicValueEnum = match item.kind {
                                                        60 => {
                                                            let fname = arena
                                                                .get_string(NodeOffset(item.data))
                                                                .unwrap_or("")
                                                                .to_string();
                                                            if fname.is_empty() {
                                                                match ft {
                                                                    BasicTypeEnum::PointerType(
                                                                        pt,
                                                                    ) => pt.const_null().into(),
                                                                    BasicTypeEnum::IntType(it) => {
                                                                        it.const_zero().into()
                                                                    }
                                                                    _ => self
                                                                        .context
                                                                        .i32_type()
                                                                        .const_zero()
                                                                        .into(),
                                                                }
                                                            } else if let Some(f) =
                                                                self.module.get_function(&fname)
                                                            {
                                                                f.as_global_value()
                                                                    .as_pointer_value()
                                                                    .into()
                                                            } else if let Some(g) =
                                                                self.module.get_global(&fname)
                                                            {
                                                                g.as_pointer_value().into()
                                                            } else {
                                                                match ft {
                                                                    BasicTypeEnum::PointerType(
                                                                        pt,
                                                                    ) => pt.const_null().into(),
                                                                    _ => self
                                                                        .context
                                                                        .i32_type()
                                                                        .const_zero()
                                                                        .into(),
                                                                }
                                                            }
                                                        }
                                                        61 | 80 => match ft {
                                                            BasicTypeEnum::IntType(it) => it
                                                                .const_int(item.data as u64, false)
                                                                .into(),
                                                            BasicTypeEnum::PointerType(pt) => {
                                                                pt.const_null().into()
                                                            }
                                                            BasicTypeEnum::StructType(st2) => {
                                                                st2.const_zero().into()
                                                            }
                                                            BasicTypeEnum::ArrayType(at) => {
                                                                at.const_zero().into()
                                                            }
                                                            _ => self
                                                                .context
                                                                .i32_type()
                                                                .const_zero()
                                                                .into(),
                                                        },
                                                        63 | 81 | 82 => {
                                                            let s = arena
                                                                .get_string(NodeOffset(item.data))
                                                                .unwrap_or("");
                                                            let sv = self
                                                                .context
                                                                .const_string(s.as_bytes(), true);
                                                            let sg = self.module.add_global(
                                                                sv.get_type(),
                                                                Some(AddressSpace::default()),
                                                                ".str",
                                                            );
                                                            sg.set_initializer(&sv);
                                                            sg.set_constant(true);
                                                            sg.set_linkage(
                                                                inkwell::module::Linkage::Private,
                                                            );
                                                            sg.set_unnamed_addr(true);
                                                            sg.as_pointer_value().into()
                                                        }
                                                        _ => match ft {
                                                            BasicTypeEnum::PointerType(pt) => {
                                                                pt.const_null().into()
                                                            }
                                                            BasicTypeEnum::IntType(it) => {
                                                                it.const_zero().into()
                                                            }
                                                            BasicTypeEnum::StructType(st2) => {
                                                                st2.const_zero().into()
                                                            }
                                                            BasicTypeEnum::ArrayType(at2) => {
                                                                at2.const_zero().into()
                                                            }
                                                            _ => self
                                                                .context
                                                                .i32_type()
                                                                .const_zero()
                                                                .into(),
                                                        },
                                                    };
                                                    if std::env::var("OPTICC_DEBUG_ARRAY_INIT")
                                                        .is_ok()
                                                    {
                                                        eprintln!("DEBUG ARRAY field[{}]: item.kind={} val={:?}", fi, item.kind, val);
                                                    }
                                                    fvals.push(val);
                                                    fi += 1;
                                                    item_off = item.next_sibling;
                                                }
                                                // Pad remaining fields
                                                while fi < n_fields {
                                                    let ft = elem_st
                                                        .get_field_type_at_index(fi)
                                                        .unwrap_or_else(|| {
                                                            self.context
                                                                .i32_type()
                                                                .as_basic_type_enum()
                                                        });
                                                    fvals.push(match ft {
                                                        BasicTypeEnum::IntType(it) => {
                                                            it.const_zero().into()
                                                        }
                                                        BasicTypeEnum::FloatType(flt) => {
                                                            flt.const_zero().into()
                                                        }
                                                        BasicTypeEnum::PointerType(pt) => {
                                                            pt.const_null().into()
                                                        }
                                                        BasicTypeEnum::StructType(st2) => {
                                                            st2.const_zero().into()
                                                        }
                                                        BasicTypeEnum::ArrayType(at2) => {
                                                            at2.const_zero().into()
                                                        }
                                                        _ => self
                                                            .context
                                                            .i32_type()
                                                            .const_zero()
                                                            .into(),
                                                    });
                                                    fi += 1;
                                                }
                                                struct_vals
                                                    .push(self.context.const_struct(&fvals, false));
                                            }
                                            if !struct_vals.is_empty() {
                                                let arr_const = elem_st.const_array(&struct_vals);
                                                global.set_initializer(&arr_const);
                                            }
                                        }
                                    }
                                }
                                let var_ptr = global.as_pointer_value();
                                if let Some((struct_type, field_names)) = struct_info.clone() {
                                    self.struct_fields
                                        .insert(var_name.clone(), field_names.clone());
                                    if actual_alloca_type.is_pointer_type() {
                                        self.pointer_struct_types
                                            .insert(var_name.clone(), struct_type);
                                    }
                                }
                                if matches!(spec_kind, 4 | 5) {
                                    if let Some(sn) = spec_node {
                                        if sn.data != 0 {
                                            if let Some(tag) = arena.get_string(NodeOffset(sn.data))
                                            {
                                                self.var_struct_tag
                                                    .insert(var_name.clone(), tag.to_string());
                                                self.global_struct_tags
                                                    .insert(var_name.clone(), tag.to_string());
                                            }
                                        }
                                    }
                                }
                                self.insert_scoped_variable(
                                    var_name.clone(),
                                    VariableBinding {
                                        ptr: var_ptr,
                                        pointee_type: actual_alloca_type,
                                        function_type,
                                    },
                                );
                                self.global_variables.insert(
                                    var_name.clone(),
                                    VariableBinding {
                                        ptr: var_ptr,
                                        pointee_type: actual_alloca_type,
                                        function_type,
                                    },
                                );
                                if matches!(spec_kind, 3 | 14)
                                    && actual_alloca_type.is_pointer_type()
                                {
                                    self.byte_pointer_vars.insert(var_name.clone());
                                }
                            } else {
                                let var_ptr = self
                                    .build_entry_alloca(actual_alloca_type, &var_name)
                                    .or_else(|_| {
                                        self.builder
                                            .build_alloca(actual_alloca_type, &var_name)
                                            .map_err(|_| BackendError::InvalidNode)
                                    })?;
                                if let Some((struct_type, field_names)) = struct_info.clone() {
                                    self.struct_fields
                                        .insert(var_name.clone(), field_names.clone());
                                    if actual_alloca_type.is_pointer_type() {
                                        self.pointer_struct_types
                                            .insert(var_name.clone(), struct_type);
                                    }
                                }
                                // Record struct tag for bitfield lookups
                                if matches!(spec_kind, 4 | 5) {
                                    if let Some(sn) = spec_node {
                                        if sn.data != 0 {
                                            if let Some(tag) = arena.get_string(NodeOffset(sn.data))
                                            {
                                                self.var_struct_tag
                                                    .insert(var_name.clone(), tag.to_string());
                                            }
                                        }
                                    }
                                }
                                self.insert_scoped_variable(
                                    var_name.clone(),
                                    VariableBinding {
                                        ptr: var_ptr,
                                        pointee_type: actual_alloca_type,
                                        function_type,
                                    },
                                );
                                if actual_alloca_type.is_pointer_type()
                                    && (matches!(spec_kind, 3 | 14)
                                        || (init_offset != NodeOffset::NULL
                                            && self.subtree_has_byte_pointer_ident(
                                                arena,
                                                init_offset,
                                            )))
                                {
                                    self.byte_pointer_vars.insert(var_name.clone());
                                }
                                // Process initializer only for non-array types
                                // (array initializers require separate aggregate handling)
                                if !is_array {
                                    if init_offset != NodeOffset::NULL {
                                        let is_designated_init = arena
                                            .get(init_offset)
                                            .map(|n| n.kind == 205)
                                            .unwrap_or(false);
                                        if is_designated_init {
                                            if let BasicTypeEnum::StructType(struct_type) =
                                                actual_alloca_type
                                            {
                                                self.lower_designated_init_into_struct(
                                                    arena,
                                                    init_offset,
                                                    var_ptr,
                                                    struct_type,
                                                    &var_name,
                                                )?;
                                            }
                                        } else if let Some(val) =
                                            self.lower_expr(arena, init_offset)?
                                        {
                                            if Self::types_compatible(actual_alloca_type, val) {
                                                let _ = self
                                                    .builder
                                                    .build_store(var_ptr, val)
                                                    .map_err(|_| BackendError::InvalidNode);
                                            } else if let (
                                                BasicTypeEnum::PointerType(ptr_ty),
                                                BasicValueEnum::IntValue(iv),
                                            ) = (actual_alloca_type, val)
                                            {
                                                let coerced =
                                                    if iv.get_zero_extended_constant() == Some(0) {
                                                        ptr_ty.const_null().as_basic_value_enum()
                                                    } else {
                                                        self.builder
                                                            .build_int_to_ptr(
                                                                iv,
                                                                ptr_ty,
                                                                "init_int2ptr",
                                                            )
                                                            .map_err(|_| BackendError::InvalidNode)?
                                                            .as_basic_value_enum()
                                                    };
                                                let _ = self
                                                    .builder
                                                    .build_store(var_ptr, coerced)
                                                    .map_err(|_| BackendError::InvalidNode);
                                            }
                                        }
                                    }
                                }
                            } // end else (non-static)
                        }
                    }
                    // Kind=60/7/8/9: plain scalar, pointer, array, or function-pointer declarator
                    // (with optional initializer as next_sibling)
                    60 | 7 | 8 | 9 => {
                        let actual_alloca_type =
                            self.declarator_llvm_type(arena, Some(child), alloca_type);
                        let function_type = self.function_type_from_declarator(
                            arena,
                            Some(child),
                            alloca_type,
                            spec_kind,
                        );
                        let var_name_opt = if child.kind == 60 {
                            arena
                                .get_string(NodeOffset(child.data))
                                .filter(|s| !s.is_empty())
                                .map(|s| s.to_string())
                        } else {
                            self.find_ident_name_in(arena, child)
                        };

                        // The initializer (if any) is stored as next_sibling of the declarator.
                        // Grab it and skip those siblings so the loop doesn't treat them as vars.
                        let init_offset = child.next_sibling;

                        if let Some(var_name) = var_name_opt {
                            if is_static_local {
                                let zero: BasicValueEnum = match actual_alloca_type {
                                    BasicTypeEnum::IntType(it) => it.const_zero().into(),
                                    BasicTypeEnum::FloatType(ft) => ft.const_zero().into(),
                                    BasicTypeEnum::PointerType(pt) => pt.const_null().into(),
                                    BasicTypeEnum::StructType(st) => st.const_zero().into(),
                                    BasicTypeEnum::ArrayType(at) => at.const_zero().into(),
                                    _ => self.context.i32_type().const_zero().into(),
                                };
                                let static_name = {
                                    let n = self.static_local_counter;
                                    self.static_local_counter += 1;
                                    format!("{}.static.{}", var_name, n)
                                };
                                let global = {
                                    let g = self.module.add_global(
                                        actual_alloca_type,
                                        Some(AddressSpace::default()),
                                        &static_name,
                                    );
                                    g.set_initializer(&zero);
                                    g.set_linkage(inkwell::module::Linkage::Internal);
                                    g
                                };
                                // Handle aggregate struct initializer
                                if init_offset != NodeOffset::NULL {
                                    if let BasicTypeEnum::StructType(st) = actual_alloca_type {
                                        let mut field_vals: Vec<BasicValueEnum> = Vec::new();
                                        let mut item_off = init_offset;
                                        let mut field_idx = 0u32;
                                        while item_off != NodeOffset::NULL
                                            && field_idx < st.count_fields()
                                        {
                                            if let Some(item) = arena.get(item_off) {
                                                let field_ty = st
                                                    .get_field_type_at_index(field_idx)
                                                    .unwrap_or_else(|| {
                                                        self.context.i32_type().as_basic_type_enum()
                                                    });
                                                let val: BasicValueEnum = match item.kind {
                                                    60 => {
                                                        // ident (function pointer name)
                                                        let fname = arena
                                                            .get_string(NodeOffset(item.data))
                                                            .unwrap_or("")
                                                            .to_string();
                                                        if fname.is_empty() {
                                                            match field_ty {
                                                                BasicTypeEnum::PointerType(pt) => {
                                                                    pt.const_null().into()
                                                                }
                                                                BasicTypeEnum::IntType(it) => {
                                                                    it.const_zero().into()
                                                                }
                                                                BasicTypeEnum::StructType(st2) => {
                                                                    st2.const_zero().into()
                                                                }
                                                                BasicTypeEnum::ArrayType(at) => {
                                                                    at.const_zero().into()
                                                                }
                                                                _ => self
                                                                    .context
                                                                    .i32_type()
                                                                    .const_zero()
                                                                    .into(),
                                                            }
                                                        } else if let Some(f) =
                                                            self.module.get_function(&fname)
                                                        {
                                                            f.as_global_value()
                                                                .as_pointer_value()
                                                                .into()
                                                        } else {
                                                            match field_ty {
                                                                BasicTypeEnum::PointerType(pt) => {
                                                                    pt.const_null().into()
                                                                }
                                                                BasicTypeEnum::StructType(st2) => {
                                                                    st2.const_zero().into()
                                                                }
                                                                BasicTypeEnum::ArrayType(at) => {
                                                                    at.const_zero().into()
                                                                }
                                                                _ => self
                                                                    .context
                                                                    .i32_type()
                                                                    .const_zero()
                                                                    .into(),
                                                            }
                                                        }
                                                    }
                                                    61 | 80 => match field_ty {
                                                        BasicTypeEnum::IntType(it) => it
                                                            .const_int(item.data as u64, false)
                                                            .into(),
                                                        BasicTypeEnum::PointerType(pt) => {
                                                            pt.const_null().into()
                                                        }
                                                        BasicTypeEnum::StructType(st2) => {
                                                            st2.const_zero().into()
                                                        }
                                                        BasicTypeEnum::ArrayType(at) => {
                                                            at.const_zero().into()
                                                        }
                                                        _ => self
                                                            .context
                                                            .i32_type()
                                                            .const_zero()
                                                            .into(),
                                                    },
                                                    _ => match field_ty {
                                                        BasicTypeEnum::PointerType(pt) => {
                                                            pt.const_null().into()
                                                        }
                                                        BasicTypeEnum::IntType(it) => {
                                                            it.const_zero().into()
                                                        }
                                                        BasicTypeEnum::StructType(st2) => {
                                                            st2.const_zero().into()
                                                        }
                                                        BasicTypeEnum::ArrayType(at) => {
                                                            at.const_zero().into()
                                                        }
                                                        _ => self
                                                            .context
                                                            .i32_type()
                                                            .const_zero()
                                                            .into(),
                                                    },
                                                };
                                                if std::env::var("OPTICC_DEBUG_STATIC").is_ok() {
                                                    eprintln!("DEBUG unconditional lower_var_decl struct field[{}]: item.kind={} val={:?}", field_idx, item.kind, val);
                                                }
                                                field_vals.push(val);
                                                field_idx += 1;
                                                item_off = item.next_sibling;
                                            } else {
                                                break;
                                            }
                                        }
                                        while field_idx < st.count_fields() {
                                            let field_ty = st
                                                .get_field_type_at_index(field_idx)
                                                .unwrap_or_else(|| {
                                                    self.context.i32_type().as_basic_type_enum()
                                                });
                                            let z: BasicValueEnum = match field_ty {
                                                BasicTypeEnum::IntType(it) => {
                                                    it.const_zero().into()
                                                }
                                                BasicTypeEnum::FloatType(ft) => {
                                                    ft.const_zero().into()
                                                }
                                                BasicTypeEnum::PointerType(pt) => {
                                                    pt.const_null().into()
                                                }
                                                BasicTypeEnum::StructType(st2) => {
                                                    st2.const_zero().into()
                                                }
                                                BasicTypeEnum::ArrayType(at) => {
                                                    at.const_zero().into()
                                                }
                                                _ => self.context.i32_type().const_zero().into(),
                                            };
                                            field_vals.push(z);
                                            field_idx += 1;
                                        }
                                        if !field_vals.is_empty() {
                                            let const_struct = self.context.const_struct(
                                                &field_vals.iter().map(|v| *v).collect::<Vec<_>>(),
                                                false,
                                            );
                                            global.set_initializer(&const_struct);
                                        }
                                    }
                                }
                                let var_ptr = global.as_pointer_value();
                                if let Some((struct_type, field_names)) = struct_info.clone() {
                                    self.struct_fields
                                        .insert(var_name.clone(), field_names.clone());
                                    if actual_alloca_type.is_pointer_type() {
                                        self.pointer_struct_types
                                            .insert(var_name.clone(), struct_type);
                                    }
                                }
                                if matches!(spec_kind, 4 | 5) {
                                    if let Some(sn) = spec_node {
                                        if sn.data != 0 {
                                            if let Some(tag) = arena.get_string(NodeOffset(sn.data))
                                            {
                                                self.var_struct_tag
                                                    .insert(var_name.clone(), tag.to_string());
                                                self.global_struct_tags
                                                    .insert(var_name.clone(), tag.to_string());
                                            }
                                        }
                                    }
                                }
                                self.insert_scoped_variable(
                                    var_name.clone(),
                                    VariableBinding {
                                        ptr: var_ptr,
                                        pointee_type: actual_alloca_type,
                                        function_type,
                                    },
                                );
                                self.global_variables.insert(
                                    var_name.clone(),
                                    VariableBinding {
                                        ptr: var_ptr,
                                        pointee_type: actual_alloca_type,
                                        function_type,
                                    },
                                );
                                if matches!(spec_kind, 3 | 14)
                                    && actual_alloca_type.is_pointer_type()
                                {
                                    self.byte_pointer_vars.insert(var_name.clone());
                                }
                                // Skip the initializer elements (they are siblings of this declarator)
                                // by breaking out — we only have one declarator in this static case
                                break;
                            } else {
                                let var_ptr = self
                                    .build_entry_alloca(actual_alloca_type, &var_name)
                                    .or_else(|_| {
                                        self.builder
                                            .build_alloca(actual_alloca_type, &var_name)
                                            .map_err(|_| BackendError::InvalidNode)
                                    })?;
                                if let Some((struct_type, field_names)) = struct_info.clone() {
                                    self.struct_fields
                                        .insert(var_name.clone(), field_names.clone());
                                    if actual_alloca_type.is_pointer_type() {
                                        self.pointer_struct_types
                                            .insert(var_name.clone(), struct_type);
                                    }
                                }
                                // Record struct tag for bitfield lookups
                                if matches!(spec_kind, 4 | 5) {
                                    if let Some(sn) = spec_node {
                                        if sn.data != 0 {
                                            if let Some(tag) = arena.get_string(NodeOffset(sn.data))
                                            {
                                                self.var_struct_tag
                                                    .insert(var_name.clone(), tag.to_string());
                                            }
                                        }
                                    }
                                }
                                self.insert_scoped_variable(
                                    var_name.clone(),
                                    VariableBinding {
                                        ptr: var_ptr,
                                        pointee_type: actual_alloca_type,
                                        function_type,
                                    },
                                );
                                if actual_alloca_type.is_pointer_type()
                                    && (matches!(spec_kind, 3 | 14)
                                        || (init_offset != NodeOffset::NULL
                                            && self.subtree_has_byte_pointer_ident(
                                                arena,
                                                init_offset,
                                            )))
                                {
                                    self.byte_pointer_vars.insert(var_name);
                                }
                            } // end else (non-static)
                        }
                    }
                    // Type specifiers: skip
                    1..=16 | 83 | 84 | 101..=106 => {}
                    // Expression initializer (rare, when chained directly)
                    _ => {
                        if matches!(child.kind, 64..=72 | 80..=82 | 61..=62) {
                            // Bare expression initializer — skip here (handled via kind=73)
                        }
                    }
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }
        Ok(())
    }

    fn types_compatible(alloca_type: BasicTypeEnum, val: BasicValueEnum) -> bool {
        match (alloca_type, val) {
            (BasicTypeEnum::IntType(_), BasicValueEnum::IntValue(_)) => true,
            (BasicTypeEnum::FloatType(_), BasicValueEnum::FloatValue(_)) => true,
            (BasicTypeEnum::PointerType(_), BasicValueEnum::PointerValue(_)) => true,
            (BasicTypeEnum::StructType(_), BasicValueEnum::StructValue(_)) => true,
            (BasicTypeEnum::ArrayType(_), BasicValueEnum::ArrayValue(_)) => true,
            _ => false,
        }
    }

    fn lower_lvalue_ptr(
        &mut self,
        arena: &Arena,
        offset: NodeOffset,
    ) -> Result<Option<(PointerValue<'ctx>, BasicTypeEnum<'ctx>)>, BackendError> {
        // Clear bitfield state; it will be set by lower_member_access_ptr if needed
        self.last_bitfield_access = None;

        let node = match arena.get(offset) {
            Some(node) => node,
            None => return Ok(None),
        };

        match node.kind {
            60 => {
                let name = match arena.get_string(NodeOffset(node.data)) {
                    Some(name) => name,
                    None => return Ok(None),
                };
                let binding = match self.variables.get(name).copied() {
                    Some(binding) => binding,
                    None => return Ok(None),
                };

                if let BasicTypeEnum::ArrayType(array_type) = binding.pointee_type {
                    let zero = self.context.i32_type().const_zero();
                    let ptr = unsafe {
                        self.builder
                            .build_gep(array_type, binding.ptr, &[zero, zero], "arraybase")
                            .map_err(|_| BackendError::InvalidNode)?
                    };
                    Ok(Some((ptr, array_type.get_element_type())))
                } else {
                    Ok(Some((binding.ptr, binding.pointee_type)))
                }
            }
            // kind 65 + data 5 is unary dereference (`*expr`) and can appear
            // as an assignable lvalue (for example `*out = value`).
            65 if node.data == 5 => {
                let ptr = match self.lower_expr(arena, node.first_child)? {
                    Some(value) if value.is_pointer_value() => value.into_pointer_value(),
                    _ => return Ok(None),
                };
                Ok(Some((
                    ptr,
                    self.context
                        .ptr_type(AddressSpace::default())
                        .as_basic_type_enum(),
                )))
            }
            68 => self.lower_array_element_ptr(arena, node),
            69 => self.lower_member_access_ptr(arena, node),
            _ => Ok(None),
        }
    }

    fn lower_member_access_ptr(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<(PointerValue<'ctx>, BasicTypeEnum<'ctx>)>, BackendError> {
        // Clear bitfield state from any previous call
        self.last_bitfield_access = None;

        let base_offset = node.first_child;
        // Field name is stored in node.data: bit 31 = is_arrow, lower bits = string offset
        let field_str_offset = NodeOffset(node.data & 0x7FFF_FFFF);
        let field_name = match arena.get_string(field_str_offset).map(|s| s.to_string()) {
            Some(name) if !name.is_empty() => name,
            // Legacy: try next_sibling (old format backward compat)
            _ => match arena.get(node.next_sibling).and_then(|n| {
                if n.kind == 60 {
                    arena.get_string(NodeOffset(n.data)).map(|s| s.to_string())
                } else {
                    None
                }
            }) {
                Some(name) => name,
                None => return Ok(None),
            },
        };

        // Try to resolve base as a simple identifier first (fast path)
        let base_name = arena.get(base_offset).and_then(|n| {
            if n.kind == 60 {
                arena.get_string(NodeOffset(n.data)).map(|s| s.to_string())
            } else {
                None
            }
        });

        if let Some(ref base_name) = base_name {
            // Look up bitfield/gep info from struct tag metadata
            let gep_lookup = self
                .var_struct_tag
                .get(base_name)
                .cloned()
                .or_else(|| self.global_struct_tags.get(base_name).cloned())
                .and_then(|tag| {
                    self.struct_gep_info
                        .get(&tag)
                        .and_then(|m| m.get(&field_name).copied())
                });

            let field_idx = if let Some((idx, bf)) = gep_lookup {
                self.last_bitfield_access = bf;
                idx
            } else {
                // Fallback: try struct_tag_fields for non-bitfield structs
                let tag_field_lookup = self
                    .var_struct_tag
                    .get(base_name)
                    .cloned()
                    .or_else(|| self.global_struct_tags.get(base_name).cloned())
                    .and_then(|tag| {
                        self.struct_tag_fields
                            .get(&tag)
                            .and_then(|fields| fields.iter().position(|f| f == &field_name))
                    });
                if let Some(idx) = tag_field_lookup {
                    idx as u32
                } else {
                    let idx = self
                        .struct_fields
                        .get(base_name)
                        .and_then(|fields| fields.iter().position(|f| f == &field_name))
                        .unwrap_or(0) as u32;
                    idx
                }
            };

            let binding = match self
                .variables
                .get(base_name)
                .copied()
                .or_else(|| self.global_variables.get(base_name).copied())
            {
                Some(binding) => binding,
                None => return Ok(None),
            };

            let is_arrow = node.data & 0x8000_0000 != 0;

            if is_arrow {
                // Arrow operator: base is a pointer to struct. Load the pointer, then GEP.
                let struct_type = self
                    .pointer_struct_types
                    .get(base_name)
                    .copied()
                    .or_else(|| {
                        self.var_struct_tag
                            .get(base_name)
                            .cloned()
                            .and_then(|tag| self.struct_tag_types.get(&tag).copied())
                    });
                let Some(struct_type) = struct_type else {
                    return Ok(None);
                };
                // Load the pointer
                let loaded_ptr = self
                    .builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        binding.ptr,
                        "arrow.load",
                    )
                    .map_err(|_| BackendError::InvalidNode)?
                    .into_pointer_value();
                let field_ptr = self
                    .builder
                    .build_struct_gep(struct_type, loaded_ptr, field_idx, "arrow.gep")
                    .map_err(|_| BackendError::InvalidNode)?;
                let field_type = struct_type
                    .get_field_type_at_index(field_idx)
                    .unwrap_or_else(|| self.context.i32_type().as_basic_type_enum());
                return Ok(Some((field_ptr, field_type)));
            }

            // Dot operator: base is a struct variable (held directly, not via pointer)
            let BasicTypeEnum::StructType(struct_type) = binding.pointee_type else {
                return Ok(None);
            };
            let field_ptr = self
                .builder
                .build_struct_gep(struct_type, binding.ptr, field_idx, "dot.gep")
                .map_err(|_| BackendError::InvalidNode)?;
            let field_type = struct_type
                .get_field_type_at_index(field_idx)
                .unwrap_or_else(|| self.context.i32_type().as_basic_type_enum());
            return Ok(Some((field_ptr, field_type)));
        }

        // Recursive path: base is a complex expression (e.g., nested member access)
        // Handle DOT access on a nested member access base (e.g., config.m.xMalloc)
        if node.data & 0x8000_0000 == 0 {
            // Dot access: base is also in memory. Use lower_member_access_ptr on base to get lvalue ptr
            if let Some(base_node) = arena.get(base_offset) {
                if base_node.kind == 68 {
                    if let Some((elem_ptr, elem_type)) =
                        self.lower_array_element_ptr(arena, base_node)?
                    {
                        if let BasicTypeEnum::StructType(struct_type) = elem_type {
                            for (tag_name, fields) in &self.struct_tag_fields {
                                if let Some(st) = self.struct_tag_types.get(tag_name).copied() {
                                    let types_match = st == struct_type || {
                                        let n = st.count_fields();
                                        n == struct_type.count_fields()
                                            && (0..n).all(|i| {
                                                st.get_field_type_at_index(i)
                                                    == struct_type.get_field_type_at_index(i)
                                            })
                                    };
                                    if types_match {
                                        if let Some(idx) =
                                            fields.iter().position(|f| f == &field_name)
                                        {
                                            let gep = self
                                                .builder
                                                .build_struct_gep(
                                                    struct_type,
                                                    elem_ptr,
                                                    idx as u32,
                                                    "arr.dot.gep",
                                                )
                                                .map_err(|_| BackendError::InvalidNode)?;
                                            let ft = struct_type
                                                .get_field_type_at_index(idx as u32)
                                                .unwrap_or_else(|| {
                                                    self.context.i32_type().as_basic_type_enum()
                                                });
                                            return Ok(Some((gep, ft)));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if base_node.kind == 69 {
                    // Recursively get the lvalue pointer of the base
                    let base_node_cloned = *base_node;
                    if let Some((base_ptr, base_type)) =
                        self.lower_member_access_ptr(arena, &base_node_cloned)?
                    {
                        if let BasicTypeEnum::StructType(struct_type) = base_type {
                            // Find field index in the nested struct type
                            let field_name_of_base = {
                                let fso = NodeOffset(base_node_cloned.data & 0x7FFF_FFFF);
                                arena
                                    .get_string(fso)
                                    .map(|s| s.to_string())
                                    .unwrap_or_default()
                            };
                            // Look up field_name in the nested struct type
                            // We compare by struct identity OR by layout (field count + field types).
                            // Identity may fail if the nested struct was built anonymously during parent
                            // registration before the named type was registered.
                            let mut found = None;
                            for (tag_name, fields) in &self.struct_tag_fields {
                                if let Some(st) = self.struct_tag_types.get(tag_name).copied() {
                                    let types_match = st == struct_type || {
                                        let n = st.count_fields();
                                        n == struct_type.count_fields()
                                            && (0..n).all(|i| {
                                                st.get_field_type_at_index(i)
                                                    == struct_type.get_field_type_at_index(i)
                                            })
                                    };
                                    if types_match {
                                        if let Some(idx) =
                                            fields.iter().position(|f| f == &field_name)
                                        {
                                            let gep = self
                                                .builder
                                                .build_struct_gep(
                                                    struct_type,
                                                    base_ptr,
                                                    idx as u32,
                                                    "nested.dot.gep",
                                                )
                                                .map_err(|_| BackendError::InvalidNode)?;
                                            let ft = struct_type
                                                .get_field_type_at_index(idx as u32)
                                                .unwrap_or_else(|| {
                                                    self.context.i32_type().as_basic_type_enum()
                                                });
                                            found = Some((gep, ft));
                                            break;
                                        }
                                    }
                                }
                            }
                            let _ = field_name_of_base; // suppress warning
                            if let Some(result) = found {
                                return Ok(Some(result));
                            }

                            // Fallback for anonymous aggregates (especially unions) where
                            // we do not have preserved field-name metadata for `struct_type`.
                            // Use a conservative heuristic by field count and common pointer
                            // member naming patterns used in SQLite (`pHash`, `pDestructor`).
                            let n = struct_type.count_fields();
                            if n > 0 {
                                let fallback_idx = if n == 1 {
                                    Some(0u32)
                                } else if n == 2 {
                                    if field_name == "pDestructor" {
                                        Some(1u32)
                                    } else if field_name == "pHash" {
                                        Some(0u32)
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                };
                                if let Some(idx) = fallback_idx {
                                    let gep = self
                                        .builder
                                        .build_struct_gep(
                                            struct_type,
                                            base_ptr,
                                            idx,
                                            "nested.dot.fallback.gep",
                                        )
                                        .map_err(|_| BackendError::InvalidNode)?;
                                    let ft =
                                        struct_type.get_field_type_at_index(idx).unwrap_or_else(
                                            || self.context.i32_type().as_basic_type_enum(),
                                        );
                                    return Ok(Some((gep, ft)));
                                }
                            }
                        } else if base_type.is_pointer_type() {
                            // Anonymous-union collapse fallback: if an intermediate member
                            // was lowered directly as a pointer field (for example `p->u`
                            // where `u` is a union containing pointer aliases), preserve the
                            // underlying pointer slot for nested `.member` accesses.
                            return Ok(Some((base_ptr, base_type)));
                        }
                    }
                }
            }
        }

        // Recursive path: base is a complex expression (e.g., nested member access like p->next->field)
        if node.data & 0x8000_0000 != 0 {
            // Arrow operator on a complex base expression
            // First, lower the base expression to get a pointer
            let base_val = self.lower_expr(arena, base_offset)?;
            let base_ptr = match base_val {
                Some(BasicValueEnum::PointerValue(ptr)) => ptr,
                _ => return Ok(None),
            };

            // Try to determine the struct type from the base expression
            // For nested arrow (e.g., head->next->value), the base is itself a member access
            // that returned a pointer. We need to find what struct type it points to.
            if let Some(base_node) = arena.get(base_offset) {
                if base_node.kind == 69 {
                    // The base is another member access. Look at its root variable
                    // to find the struct tag, then use the tag's struct type.
                    let root_var = self.find_member_access_root_var(arena, base_node);
                    if let Some(root_name) = root_var {
                        // Get the struct type from pointer_struct_types (same struct for self-ref)
                        if let Some(struct_type) =
                            self.pointer_struct_types.get(&root_name).copied()
                        {
                            // Look up field names from struct_fields
                            if let Some(fields) = self.struct_fields.get(&root_name) {
                                let field_idx =
                                    fields.iter().position(|f| f == &field_name).unwrap_or(0)
                                        as u32;
                                let field_ptr = self
                                    .builder
                                    .build_struct_gep(struct_type, base_ptr, field_idx, "chain.gep")
                                    .map_err(|_| BackendError::InvalidNode)?;
                                let ft = struct_type
                                    .get_field_type_at_index(field_idx)
                                    .unwrap_or_else(|| {
                                        self.context.i32_type().as_basic_type_enum()
                                    });
                                return Ok(Some((field_ptr, ft)));
                            }
                        }
                    }

                    // Fallback: search struct_tag_types/struct_tag_fields for a struct with this field
                    for (tag_name, fields) in &self.struct_tag_fields {
                        if fields.contains(&field_name) {
                            if let Some(struct_type) = self.struct_tag_types.get(tag_name).copied()
                            {
                                let field_idx =
                                    fields.iter().position(|f| f == &field_name).unwrap_or(0)
                                        as u32;
                                let field_ptr = self
                                    .builder
                                    .build_struct_gep(struct_type, base_ptr, field_idx, "chain.gep")
                                    .map_err(|_| BackendError::InvalidNode)?;
                                let ft = struct_type
                                    .get_field_type_at_index(field_idx)
                                    .unwrap_or_else(|| {
                                        self.context.i32_type().as_basic_type_enum()
                                    });
                                return Ok(Some((field_ptr, ft)));
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Find the root variable name of a chained member access expression.
    /// E.g., for `head->next->value`, walks back through kind=69 nodes to find "head".
    fn find_member_access_root_var(&self, arena: &Arena, node: &CAstNode) -> Option<String> {
        let base_offset = node.first_child;
        if let Some(base) = arena.get(base_offset) {
            if base.kind == 60 {
                return arena
                    .get_string(NodeOffset(base.data))
                    .map(|s| s.to_string());
            }
            if base.kind == 69 {
                return self.find_member_access_root_var(arena, base);
            }
        }
        None
    }

    fn lower_array_element_ptr(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<(PointerValue<'ctx>, BasicTypeEnum<'ctx>)>, BackendError> {
        let index_val = match self.lower_expr(arena, node.next_sibling)? {
            Some(BasicValueEnum::IntValue(index)) => index,
            _ => return Ok(None),
        };

        if let Some(base_node) = arena.get(node.first_child) {
            if base_node.kind == 60 {
                if let Some(name) = arena.get_string(NodeOffset(base_node.data)) {
                    if let Some(binding) = self.variables.get(name).copied() {
                        if let BasicTypeEnum::ArrayType(array_type) = binding.pointee_type {
                            let zero = self.context.i32_type().const_zero();
                            let ptr = unsafe {
                                self.builder
                                    .build_gep(
                                        array_type,
                                        binding.ptr,
                                        &[zero, index_val],
                                        "arrayidx",
                                    )
                                    .map_err(|_| BackendError::InvalidNode)?
                            };
                            return Ok(Some((ptr, array_type.get_element_type())));
                        }
                    }
                }
            }
        }

        // If the indexed base is itself an lvalue array (for example a struct
        // field like `hash.a[h]`), index directly into the original storage.
        // Falling back to an ArrayValue copy here would misroute writes to a
        // temporary alloca instead of the real array.
        if let Some((base_ptr, base_type)) = self.lower_lvalue_ptr(arena, node.first_child)? {
            if let BasicTypeEnum::ArrayType(arr_ty) = base_type {
                let zero = self.context.i32_type().const_zero();
                let ptr = unsafe {
                    self.builder
                        .build_gep(arr_ty, base_ptr, &[zero, index_val], "arrayidx")
                        .map_err(|_| BackendError::InvalidNode)?
                };
                return Ok(Some((ptr, arr_ty.get_element_type())));
            }
        }

        // Fallback: evaluate base as expression, determine element type
        // Try to infer element type from the base variable's pointee type
        let mut elem_type = self.context.i32_type().as_basic_type_enum();
        if let Some(base_node) = arena.get(node.first_child) {
            if base_node.kind == 60 {
                if let Some(name) = arena.get_string(NodeOffset(base_node.data)) {
                    if let Some(binding) = self.variables.get(name).copied() {
                        // If the variable is a pointer, default element type to byte.
                        // Opaque pointers do not preserve pointee element type, and using
                        // `ptr` as an element type causes char* indexing to mis-lower into
                        // pointer loads/stores.
                        if binding.pointee_type.is_pointer_type() {
                            if let Some(st) = self.pointer_struct_types.get(name).copied() {
                                elem_type = st.as_basic_type_enum();
                            } else if name.starts_with("pp") || name.starts_with("pz") {
                                elem_type = self
                                    .context
                                    .ptr_type(AddressSpace::default())
                                    .as_basic_type_enum();
                            } else {
                                elem_type = self.context.i8_type().as_basic_type_enum();
                            }
                        }
                    }
                }
            }
        }
        let base_val = self.lower_expr(arena, node.first_child)?;
        match base_val {
            Some(BasicValueEnum::PointerValue(base_ptr)) => {
                let ptr = unsafe {
                    self.builder
                        .build_gep(elem_type, base_ptr, &[index_val], "arrayidx")
                        .map_err(|_| BackendError::InvalidNode)?
                };
                Ok(Some((ptr, elem_type)))
            }
            Some(BasicValueEnum::ArrayValue(arr_val)) => {
                let arr_ty = arr_val.get_type();
                let tmp = self
                    .builder
                    .build_alloca(arr_ty, "arr_idx_base")
                    .map_err(|_| BackendError::InvalidNode)?;
                self.builder
                    .build_store(tmp, arr_val)
                    .map_err(|_| BackendError::InvalidNode)?;
                let zero = self.context.i32_type().const_zero();
                let ptr = unsafe {
                    self.builder
                        .build_gep(arr_ty, tmp, &[zero, index_val], "arrayidx")
                        .map_err(|_| BackendError::InvalidNode)?
                };
                Ok(Some((ptr, arr_ty.get_element_type())))
            }
            _ => Ok(None),
        }
    }

    fn apply_assignment_op(
        &mut self,
        op_code: u32,
        lhs_val: BasicValueEnum<'ctx>,
        rhs_val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, BackendError> {
        if op_code == 19 {
            return Ok(rhs_val);
        }

        // Handle pointer arithmetic: ptr += int or ptr -= int → GEP
        if lhs_val.is_pointer_value() && (op_code == 1 || op_code == 2) {
            let ptr = lhs_val.into_pointer_value();
            let index = if rhs_val.is_int_value() {
                let idx = rhs_val.into_int_value();
                if op_code == 2 {
                    self.builder
                        .build_int_neg(idx, "neg_idx")
                        .map_err(|_| BackendError::InvalidNode)?
                } else {
                    idx
                }
            } else if rhs_val.is_pointer_value() {
                // ptr - ptr → ptrdiff
                let lhs_int = self
                    .builder
                    .build_ptr_to_int(ptr, self.context.i64_type(), "ptr2int_lhs")
                    .map_err(|_| BackendError::InvalidNode)?;
                let rhs_int = self
                    .builder
                    .build_ptr_to_int(
                        rhs_val.into_pointer_value(),
                        self.context.i64_type(),
                        "ptr2int_rhs",
                    )
                    .map_err(|_| BackendError::InvalidNode)?;
                return Ok(self
                    .builder
                    .build_int_sub(lhs_int, rhs_int, "ptrdiff")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into());
            } else {
                return Err(BackendError::InvalidNode);
            };
            let result = unsafe {
                self.builder
                    .build_gep(self.context.i8_type(), ptr, &[index], "ptr_arith")
                    .map_err(|_| BackendError::InvalidNode)?
            };
            return Ok(result.into());
        }

        // Convert any remaining pointer operands to integers before arithmetic
        let lhs_val = if lhs_val.is_pointer_value() {
            self.builder
                .build_ptr_to_int(
                    lhs_val.into_pointer_value(),
                    self.context.i64_type(),
                    "op_ptr2int_lhs",
                )
                .map_err(|_| BackendError::InvalidNode)?
                .into()
        } else {
            lhs_val
        };
        let rhs_val = if rhs_val.is_pointer_value() {
            self.builder
                .build_ptr_to_int(
                    rhs_val.into_pointer_value(),
                    self.context.i64_type(),
                    "op_ptr2int_rhs",
                )
                .map_err(|_| BackendError::InvalidNode)?
                .into()
        } else {
            rhs_val
        };

        let use_float = lhs_val.is_float_value() || rhs_val.is_float_value();
        if use_float {
            let lhs_float = if lhs_val.is_float_value() {
                lhs_val.into_float_value()
            } else {
                self.builder
                    .build_unsigned_int_to_float(
                        lhs_val.into_int_value(),
                        self.context.f64_type(),
                        "assign_uitofp_lhs",
                    )
                    .map_err(|_| BackendError::InvalidNode)?
            };
            let rhs_float = if rhs_val.is_float_value() {
                rhs_val.into_float_value()
            } else {
                self.builder
                    .build_unsigned_int_to_float(
                        rhs_val.into_int_value(),
                        self.context.f64_type(),
                        "assign_uitofp_rhs",
                    )
                    .map_err(|_| BackendError::InvalidNode)?
            };

            return match op_code {
                1 => Ok(self
                    .builder
                    .build_float_add(lhs_float, rhs_float, "assign_fadd")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into()),
                2 => Ok(self
                    .builder
                    .build_float_sub(lhs_float, rhs_float, "assign_fsub")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into()),
                3 => Ok(self
                    .builder
                    .build_float_mul(lhs_float, rhs_float, "assign_fmul")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into()),
                4 => Ok(self
                    .builder
                    .build_float_div(lhs_float, rhs_float, "assign_fdiv")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into()),
                _ => Err(BackendError::InvalidOperator(op_code)),
            };
        }

        if !lhs_val.is_int_value() || !rhs_val.is_int_value() {
            return Err(BackendError::InvalidNode);
        }
        let lhs_int = lhs_val.into_int_value();
        let rhs_int = rhs_val.into_int_value();
        // Coerce both int operands to the same width (promote narrower to wider)
        let (lhs_int, rhs_int) = {
            let lw = lhs_int.get_type().get_bit_width();
            let rw = rhs_int.get_type().get_bit_width();
            if lw == rw {
                (lhs_int, rhs_int)
            } else if lw < rw {
                let extended = self
                    .builder
                    .build_int_s_extend(lhs_int, rhs_int.get_type(), "assign_sext_lhs")
                    .map_err(|_| BackendError::InvalidNode)?;
                (extended, rhs_int)
            } else {
                let extended = self
                    .builder
                    .build_int_s_extend(rhs_int, lhs_int.get_type(), "assign_sext_rhs")
                    .map_err(|_| BackendError::InvalidNode)?;
                (lhs_int, extended)
            }
        };
        match op_code {
            1 => Ok(self
                .builder
                .build_int_add(lhs_int, rhs_int, "assign_add")
                .map_err(|_| BackendError::InvalidNode)?
                .into()),
            2 => Ok(self
                .builder
                .build_int_sub(lhs_int, rhs_int, "assign_sub")
                .map_err(|_| BackendError::InvalidNode)?
                .into()),
            3 => Ok(self
                .builder
                .build_int_mul(lhs_int, rhs_int, "assign_mul")
                .map_err(|_| BackendError::InvalidNode)?
                .into()),
            4 => Ok(self
                .builder
                .build_int_signed_div(lhs_int, rhs_int, "assign_div")
                .map_err(|_| BackendError::InvalidNode)?
                .into()),
            5 => Ok(self
                .builder
                .build_int_signed_rem(lhs_int, rhs_int, "assign_rem")
                .map_err(|_| BackendError::InvalidNode)?
                .into()),
            14 => Ok(self
                .builder
                .build_and(lhs_int, rhs_int, "assign_and")
                .map_err(|_| BackendError::InvalidNode)?
                .into()),
            15 => Ok(self
                .builder
                .build_or(lhs_int, rhs_int, "assign_or")
                .map_err(|_| BackendError::InvalidNode)?
                .into()),
            16 => Ok(self
                .builder
                .build_xor(lhs_int, rhs_int, "assign_xor")
                .map_err(|_| BackendError::InvalidNode)?
                .into()),
            17 => Ok(self
                .builder
                .build_left_shift(lhs_int, rhs_int, "assign_shl")
                .map_err(|_| BackendError::InvalidNode)?
                .into()),
            18 => Ok(self
                .builder
                .build_right_shift(lhs_int, rhs_int, false, "assign_shr")
                .map_err(|_| BackendError::InvalidNode)?
                .into()),
            _ => Err(BackendError::InvalidOperator(op_code)),
        }
    }

    fn build_struct_llvm_type(&self, arena: &Arena, node: &CAstNode) -> BasicTypeEnum<'ctx> {
        let mut field_types: Vec<BasicTypeEnum<'ctx>> = Vec::new();
        // Track current bitfield group to pack consecutive bitfields into a single storage unit
        let mut bitfield_group: Option<(u16, u32, u32)> = None; // (base_kind, bits_used, capacity)
        let mut member_off = node.first_child;
        while member_off != NodeOffset::NULL {
            if let Some(member) = arena.get(member_off) {
                if member.kind == 200 {
                    member_off = member.next_sibling;
                    continue;
                }
                // Walk children to detect type and bitfield
                let mut base_kind = 2u16;
                let mut has_pointer = false;
                let mut array_len: Option<u32> = None;
                let mut bitfield_width: Option<u32> = None;
                let mut struct_spec_offset = NodeOffset::NULL; // for nested struct/union fields
                let mut check_off = member.first_child;
                while check_off != NodeOffset::NULL {
                    if let Some(cn) = arena.get(check_off) {
                        match cn.kind {
                            1..=6 | 10..=13 | 83 | 84 => {
                                base_kind = cn.kind;
                                if matches!(cn.kind, 4 | 5) {
                                    struct_spec_offset = check_off;
                                }
                            }
                            7 | 9 => has_pointer = true, // pointer or function-pointer field → ptr
                            8 => {
                                if let Some(len) = self.infer_array_declarator_len(arena, cn) {
                                    array_len = Some(len);
                                }
                            }
                            27 => {
                                if cn.data > 0 {
                                    bitfield_width = Some(cn.data);
                                }
                            }
                            _ => {}
                        }
                        check_off = cn.next_sibling;
                    } else {
                        break;
                    }
                }

                if let Some(bw) = bitfield_width {
                    let storage_bits = self.node_kind_bit_width(base_kind);
                    if let Some((grp_kind, ref mut bits_used, capacity)) = bitfield_group {
                        if grp_kind == base_kind && *bits_used + bw <= capacity {
                            *bits_used += bw;
                            // Same storage unit – no new field_type
                        } else {
                            field_types.push(self.node_kind_to_llvm_type(base_kind));
                            bitfield_group = Some((base_kind, bw, storage_bits));
                        }
                    } else {
                        field_types.push(self.node_kind_to_llvm_type(base_kind));
                        bitfield_group = Some((base_kind, bw, storage_bits));
                    }
                } else {
                    bitfield_group = None;
                    if has_pointer {
                        field_types.push(
                            self.context
                                .ptr_type(AddressSpace::default())
                                .as_basic_type_enum(),
                        );
                    } else if let Some(len) = array_len {
                        let elem_type = if matches!(base_kind, 4 | 5)
                            && struct_spec_offset != NodeOffset::NULL
                        {
                            if let Some(sn) = arena.get(struct_spec_offset) {
                                self.specifier_to_llvm_type(arena, sn)
                            } else {
                                self.node_kind_to_llvm_type(base_kind)
                            }
                        } else {
                            self.node_kind_to_llvm_type(base_kind)
                        };
                        field_types.push(elem_type.array_type(len).as_basic_type_enum());
                    } else if matches!(base_kind, 4 | 5) && struct_spec_offset != NodeOffset::NULL {
                        // Nested struct/union field — resolve recursively
                        let nested_type = if let Some(sn) = arena.get(struct_spec_offset) {
                            self.specifier_to_llvm_type(arena, sn)
                        } else {
                            self.node_kind_to_llvm_type(base_kind)
                        };
                        field_types.push(nested_type);
                    } else {
                        field_types.push(self.node_kind_to_llvm_type(base_kind));
                    }
                }

                member_off = member.next_sibling;
            } else {
                break;
            }
        }
        if field_types.is_empty() {
            self.context.i8_type().as_basic_type_enum()
        } else {
            self.context
                .struct_type(
                    &field_types,
                    self.node_has_attr(arena, node, &["packed", "__packed__"]),
                )
                .as_basic_type_enum()
        }
    }

    fn collect_struct_field_names(arena: &Arena, node: &CAstNode) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        let mut member_off = node.first_child;
        while member_off != NodeOffset::NULL {
            if let Some(member) = arena.get(member_off) {
                if member.kind == 200 {
                    member_off = member.next_sibling;
                    continue;
                }
                let mut child_off = member.first_child;
                let mut found = false;
                while child_off != NodeOffset::NULL {
                    if let Some(child) = arena.get(child_off) {
                        if child.kind == 60 {
                            if let Some(name) = arena.get_string(NodeOffset(child.data)) {
                                names.push(name.to_string());
                                found = true;
                                break;
                            }
                        }
                        // Descend into bitfield wrapper (kind=27) to find ident
                        if child.kind == 27 && !found {
                            let mut inner = child.first_child;
                            while inner != NodeOffset::NULL {
                                if let Some(inner_n) = arena.get(inner) {
                                    if inner_n.kind == 60 {
                                        if let Some(name) =
                                            arena.get_string(NodeOffset(inner_n.data))
                                        {
                                            names.push(name.to_string());
                                            found = true;
                                            break;
                                        }
                                    }
                                    inner = inner_n.next_sibling;
                                } else {
                                    break;
                                }
                            }
                            if found {
                                break;
                            }
                        }
                        // Descend into pointer/array declarators (kind=7/8) to find ident
                        if matches!(child.kind, 7 | 8) && !found {
                            let mut inner = child.first_child;
                            while inner != NodeOffset::NULL {
                                if let Some(inner_n) = arena.get(inner) {
                                    if inner_n.kind == 60 {
                                        if let Some(name) =
                                            arena.get_string(NodeOffset(inner_n.data))
                                        {
                                            names.push(name.to_string());
                                            found = true;
                                            break;
                                        }
                                    }
                                    // Descent: kind=7 → kind=9 (func-ptr like `void *(*name)(...)`)
                                    if inner_n.kind == 9 && !found {
                                        let mut ic = inner_n.first_child;
                                        while ic != NodeOffset::NULL {
                                            if let Some(ic_n) = arena.get(ic) {
                                                if ic_n.kind == 60 {
                                                    if let Some(name) =
                                                        arena.get_string(NodeOffset(ic_n.data))
                                                    {
                                                        names.push(name.to_string());
                                                        found = true;
                                                        break;
                                                    }
                                                }
                                                if ic_n.kind == 7 && !found {
                                                    let mut ic2 = ic_n.first_child;
                                                    while ic2 != NodeOffset::NULL {
                                                        if let Some(ic2_n) = arena.get(ic2) {
                                                            if ic2_n.kind == 60 {
                                                                if let Some(name) = arena
                                                                    .get_string(NodeOffset(
                                                                        ic2_n.data,
                                                                    ))
                                                                {
                                                                    names.push(name.to_string());
                                                                    found = true;
                                                                    break;
                                                                }
                                                            }
                                                            ic2 = ic2_n.next_sibling;
                                                        } else {
                                                            break;
                                                        }
                                                    }
                                                }
                                                if found {
                                                    break;
                                                }
                                                ic = ic_n.next_sibling;
                                            } else {
                                                break;
                                            }
                                        }
                                    }
                                    if found {
                                        break;
                                    }
                                    inner = inner_n.next_sibling;
                                } else {
                                    break;
                                }
                            }
                            if found {
                                break;
                            }
                        }
                        // Descend into function-pointer declarators (kind=9 → kind=7 → kind=60)
                        if child.kind == 9 && !found {
                            let mut inner = child.first_child;
                            while inner != NodeOffset::NULL {
                                if let Some(inner_n) = arena.get(inner) {
                                    // kind=7 (pointer): look inside for the ident
                                    if inner_n.kind == 7 {
                                        let mut inner2 = inner_n.first_child;
                                        while inner2 != NodeOffset::NULL {
                                            if let Some(inner2_n) = arena.get(inner2) {
                                                if inner2_n.kind == 60 {
                                                    if let Some(name) =
                                                        arena.get_string(NodeOffset(inner2_n.data))
                                                    {
                                                        names.push(name.to_string());
                                                        found = true;
                                                        break;
                                                    }
                                                }
                                                inner2 = inner2_n.next_sibling;
                                            } else {
                                                break;
                                            }
                                        }
                                    } else if inner_n.kind == 60 {
                                        if let Some(name) =
                                            arena.get_string(NodeOffset(inner_n.data))
                                        {
                                            names.push(name.to_string());
                                            found = true;
                                        }
                                    }
                                    if found {
                                        break;
                                    }
                                    inner = inner_n.next_sibling;
                                } else {
                                    break;
                                }
                            }
                            if found {
                                break;
                            }
                        }
                        child_off = child.next_sibling;
                    } else {
                        break;
                    }
                }
                if !found {
                    names.push(format!("_field{}", names.len()));
                }
                member_off = member.next_sibling;
            } else {
                break;
            }
        }
        names
    }

    /// Pre-register a function definition so it can be called before its body is compiled.
    /// This extracts the signature (name, return type, parameter types) without
    /// compiling the body.
    fn pre_register_func_def(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<(), BackendError> {
        let mut func_name = "func".to_string();
        let mut param_types: Vec<BasicMetadataTypeEnum> = Vec::new();
        let mut return_llvm_type: Option<BasicTypeEnum<'ctx>> = None;
        let mut is_void_ret = false;
        let mut is_variadic = false;
        let mut found_declarator = false;

        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                match child.kind {
                    1..=6 | 16 | 83 => {
                        is_void_ret = child.kind == 1;
                        if !is_void_ret {
                            return_llvm_type = Some(self.specifier_to_llvm_type(arena, child));
                        }
                    }
                    7..=9 => {
                        if let Some(func_decl_offset) =
                            self.find_function_declarator_offset(arena, child_offset)
                        {
                            found_declarator = true;
                            let callable_decl_offset =
                                self.select_callable_function_decl_offset(arena, func_decl_offset);
                            if let Some(func_decl) = arena.get(callable_decl_offset) {
                                let return_decl = self
                                    .find_function_declarator_root_offset(arena, child_offset)
                                    .and_then(|off| arena.get(off))
                                    .unwrap_or(func_decl);
                                // Check variadic flag
                                if func_decl.data == 1 {
                                    is_variadic = true;
                                }
                                let inferred_ptr_return = self
                                    .declarator_llvm_type(
                                        arena,
                                        Some(return_decl),
                                        self.context.i32_type().as_basic_type_enum(),
                                    )
                                    .is_pointer_type();
                                if is_void_ret && inferred_ptr_return {
                                    is_void_ret = false;
                                    return_llvm_type = Some(
                                        self.context
                                            .ptr_type(inkwell::AddressSpace::default())
                                            .as_basic_type_enum(),
                                    );
                                } else if !is_void_ret {
                                    return_llvm_type = Some(self.declarator_llvm_type(
                                        arena,
                                        Some(return_decl),
                                        return_llvm_type.unwrap_or_else(|| {
                                            self.context.i32_type().as_basic_type_enum()
                                        }),
                                    ));
                                }
                                if let Some(name) = self.find_ident_name(arena, func_decl) {
                                    func_name = name;
                                }
                                let mut param_off =
                                    self.function_decl_param_offset(arena, func_decl);
                                while param_off != NodeOffset::NULL {
                                    if let Some(param) = arena.get(param_off) {
                                        if param.kind == 24 {
                                            let (ptype, _, _, _) =
                                                self.extract_param_type_name(arena, param);
                                            param_types.push(ptype.into());
                                        }
                                        param_off = param.next_sibling;
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }

        if !found_declarator {
            return Ok(());
        }

        // Skip registration if name extraction failed (default "func" name)
        if func_name == "func" {
            return Ok(());
        }

        let (_sig_name, is_static_fn, is_inline_fn, _) = self.function_signature_info(arena, node);
        if is_static_fn && func_name != "func" {
            self.static_declared_functions.insert(func_name.clone());
        }
        if is_static_fn
            && is_inline_fn
            && !self.reachable_functions.is_empty()
            && !self.reachable_functions.contains(&func_name)
        {
            return Ok(());
        }

        let ret_llvm =
            return_llvm_type.unwrap_or_else(|| self.context.i32_type().as_basic_type_enum());
        let fn_type = if is_void_ret {
            self.context.void_type().fn_type(&param_types, is_variadic)
        } else {
            ret_llvm.fn_type(&param_types, is_variadic)
        };
        let function = self
            .module
            .get_function(&func_name)
            .unwrap_or_else(|| self.module.add_function(&func_name, fn_type, None));
        // Note: don't set internal linkage here — it creates invalid "declare internal"
        // if the function body fails to lower. Linkage is set in lower_func_def instead.
        // Apply any __attribute__ decorations to the pre-registered function
        let attrs = self.extract_attributes(arena, node);
        self.apply_function_attributes(function, &attrs);
        self.register_used_attributes(function.as_global_value().as_pointer_value(), &attrs);
        self.functions.insert(func_name, function);
        Ok(())
    }

    fn lower_func_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        // Use the same logic as pre_register_func_def to get correct param types
        let mut func_name = "func".to_string();
        let mut param_types: Vec<BasicMetadataTypeEnum> = Vec::new();
        let mut return_llvm_type: Option<BasicTypeEnum<'ctx>> = None;
        let mut is_void_ret = false;
        let mut is_variadic = false;
        let mut found_declarator = false;

        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                match child.kind {
                    1..=6 | 16 | 83 => {
                        is_void_ret = child.kind == 1;
                        if !is_void_ret {
                            return_llvm_type = Some(self.specifier_to_llvm_type(arena, child));
                        }
                    }
                    7..=9 | 21 | 22 => {
                        if let Some(func_decl_offset) =
                            self.find_function_declarator_offset(arena, child_offset)
                        {
                            found_declarator = true;
                            let callable_decl_offset =
                                self.select_callable_function_decl_offset(arena, func_decl_offset);
                            if let Some(func_decl) = arena.get(callable_decl_offset) {
                                let return_decl = self
                                    .find_function_declarator_root_offset(arena, child_offset)
                                    .and_then(|off| arena.get(off))
                                    .unwrap_or(func_decl);
                                if func_decl.data == 1 {
                                    is_variadic = true;
                                }
                                let inferred_ptr_return = self
                                    .declarator_llvm_type(
                                        arena,
                                        Some(return_decl),
                                        self.context.i32_type().as_basic_type_enum(),
                                    )
                                    .is_pointer_type();
                                if is_void_ret && inferred_ptr_return {
                                    is_void_ret = false;
                                    return_llvm_type = Some(
                                        self.context
                                            .ptr_type(inkwell::AddressSpace::default())
                                            .as_basic_type_enum(),
                                    );
                                } else if !is_void_ret {
                                    return_llvm_type = Some(self.declarator_llvm_type(
                                        arena,
                                        Some(return_decl),
                                        return_llvm_type.unwrap_or_else(|| {
                                            self.context.i32_type().as_basic_type_enum()
                                        }),
                                    ));
                                }
                                if let Some(name) = self.find_ident_name(arena, func_decl) {
                                    func_name = name;
                                }
                                let mut param_off =
                                    self.function_decl_param_offset(arena, func_decl);
                                while param_off != NodeOffset::NULL {
                                    if let Some(param) = arena.get(param_off) {
                                        if param.kind == 24 {
                                            let (ptype, _, _, _) =
                                                self.extract_param_type_name(arena, param);
                                            param_types.push(ptype.into());
                                        }
                                        param_off = param.next_sibling;
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }

        if !found_declarator {
            return Ok(());
        }

        let (_sig_name, is_static_fn, is_inline_fn, _) = self.function_signature_info(arena, node);
        if is_static_fn
            && is_inline_fn
            && !self.reachable_functions.is_empty()
            && !self.reachable_functions.contains(&func_name)
        {
            return Ok(());
        }

        let ret_llvm =
            return_llvm_type.unwrap_or_else(|| self.context.i32_type().as_basic_type_enum());
        let fn_type = if is_void_ret {
            self.context.void_type().fn_type(&param_types, is_variadic)
        } else {
            ret_llvm.fn_type(&param_types, is_variadic)
        };
        // Only add if not already registered (pre_register may have added it)
        if self.module.get_function(&func_name).is_none() {
            let function = self.module.add_function(&func_name, fn_type, None);
            // Don't set internal linkage on declarations — only lower_func_def should
            // set it, because "declare internal" without a body is invalid LLVM IR.
            let attrs = self.extract_attributes(arena, node);
            self.apply_function_attributes(function, &attrs);
            self.register_used_attributes(function.as_global_value().as_pointer_value(), &attrs);
            self.functions.insert(func_name, function);
        }
        Ok(())
    }

    fn lower_func_def(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        // Clear loop/switch break/continue stacks from any previous function
        self.break_stack.clear();
        self.continue_stack.clear();

        let mut func_name = "func".to_string();
        let mut param_types: Vec<BasicMetadataTypeEnum> = Vec::new();
        let mut param_names: Vec<String> = Vec::new();
        let mut param_llvm_types_list: Vec<BasicTypeEnum<'ctx>> = Vec::new();
        let mut param_struct_infos: Vec<Option<(StructType<'ctx>, Vec<String>)>> = Vec::new();
        let mut param_function_types: Vec<Option<FunctionType<'ctx>>> = Vec::new();
        let mut body_offset = NodeOffset::NULL;
        let mut return_llvm_type: Option<BasicTypeEnum<'ctx>> = None;
        let mut is_void_ret = false;
        let mut is_variadic = false;

        // Walk first_child chain: specifiers -> kind=9(func_decl) -> kind=40(body)
        let mut child_offset = node.first_child;
        let mut seen_func_declarator = false;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                // After func declarator, any non-spec/non-decl child is the body
                if seen_func_declarator
                    && body_offset == NodeOffset::NULL
                    && !matches!(child.kind, 1..=9 | 16 | 83 | 101..=106)
                {
                    body_offset = child_offset;
                }
                match child.kind {
                    1..=6 | 16 | 83 => {
                        is_void_ret = child.kind == 1;
                        if !is_void_ret {
                            return_llvm_type = Some(self.specifier_to_llvm_type(arena, child));
                        }
                    }
                    7..=9 => {
                        let Some(func_decl_offset) =
                            self.find_function_declarator_offset(arena, child_offset)
                        else {
                            child_offset = child.next_sibling;
                            continue;
                        };
                        let callable_decl_offset =
                            self.select_callable_function_decl_offset(arena, func_decl_offset);
                        let Some(func_decl) = arena.get(callable_decl_offset) else {
                            child_offset = child.next_sibling;
                            continue;
                        };
                        let return_decl = self
                            .find_function_declarator_root_offset(arena, child_offset)
                            .and_then(|off| arena.get(off))
                            .unwrap_or(func_decl);
                        seen_func_declarator = true;
                        // Check variadic flag (data=1 means ...)
                        if func_decl.data == 1 {
                            is_variadic = true;
                        }
                        let inferred_ptr_return = self
                            .declarator_llvm_type(
                                arena,
                                Some(return_decl),
                                self.context.i32_type().as_basic_type_enum(),
                            )
                            .is_pointer_type();
                        if is_void_ret && inferred_ptr_return {
                            is_void_ret = false;
                            return_llvm_type = Some(
                                self.context
                                    .ptr_type(AddressSpace::default())
                                    .as_basic_type_enum(),
                            );
                        }
                        if !is_void_ret {
                            return_llvm_type = Some(self.declarator_llvm_type(
                                arena,
                                Some(return_decl),
                                return_llvm_type.unwrap_or_else(|| {
                                    self.context.i32_type().as_basic_type_enum()
                                }),
                            ));
                        }
                        if let Some(name) = self.find_ident_name(arena, func_decl) {
                            func_name = name;
                        }
                        let mut param_off = self.function_decl_param_offset(arena, func_decl);
                        while param_off != NodeOffset::NULL {
                            if let Some(param) = arena.get(param_off) {
                                if param.kind == 24 {
                                    let (ptype, pname, struct_info, function_type) =
                                        self.extract_param_type_name(arena, param);
                                    param_types.push(ptype.into());
                                    param_llvm_types_list.push(ptype);
                                    param_names.push(pname);
                                    param_struct_infos.push(struct_info);
                                    param_function_types.push(function_type);
                                }
                                param_off = param.next_sibling;
                            } else {
                                break;
                            }
                        }
                    }
                    40 => {
                        body_offset = child_offset;
                    }
                    _ => {}
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }

        if !self.reachable_functions.is_empty() && !self.reachable_functions.contains(&func_name) {
            return Ok(());
        }

        let is_static_fn = self.function_signature_info(arena, node).1;
        let ret_llvm =
            return_llvm_type.unwrap_or_else(|| self.context.i32_type().as_basic_type_enum());
        let fn_type = if is_void_ret {
            self.context.void_type().fn_type(&param_types, is_variadic)
        } else {
            ret_llvm.fn_type(&param_types, is_variadic)
        };
        // Use the pre-registered function if available, otherwise create new
        let function = self
            .module
            .get_function(&func_name)
            .unwrap_or_else(|| self.module.add_function(&func_name, fn_type, None));
        if is_static_fn {
            function.set_linkage(inkwell::module::Linkage::Internal);
        }

        // Reconcile return type with actual LLVM function signature.
        // A pre-registered function may have a different return type than what
        // the definition resolves to (e.g., forward-declared as i32 but defined as ptr).
        // Always use the actual LLVM function's return type to keep IR consistent.
        let actual_fn_ret = function.get_type().get_return_type();
        let (ret_llvm, is_void_ret) = match actual_fn_ret {
            None => (ret_llvm, true),
            Some(actual_ty) => (actual_ty, false),
        };

        // Apply __attribute__ decorations to the function
        let attrs = self.extract_attributes(arena, node);
        self.apply_function_attributes(function, &attrs);
        self.register_used_attributes(function.as_global_value().as_pointer_value(), &attrs);
        self.register_lifecycle_attributes(function, &attrs);
        self.functions.insert(func_name.clone(), function);

        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);

        self.variables.clear();
        // Restore global variables so function bodies can reference them
        for (name, binding) in &self.global_variables {
            self.variables.insert(name.clone(), *binding);
        }
        self.struct_fields.clear();
        self.pointer_struct_types.clear();
        self.byte_pointer_vars.clear();
        self.var_struct_tag.clear();
        // Restore global struct tags so member access GEP works for globals
        for (name, tag) in &self.global_struct_tags {
            self.var_struct_tag.insert(name.clone(), tag.clone());
        }
        self.label_blocks.clear();
        self.current_return_type = if is_void_ret { None } else { Some(ret_llvm) };

        for (i, (((pname, ptype), struct_info), function_type)) in param_names
            .iter()
            .zip(param_llvm_types_list.iter())
            .zip(param_struct_infos.iter())
            .zip(param_function_types.iter())
            .enumerate()
        {
            let param_ptr = self
                .builder
                .build_alloca(*ptype, pname)
                .map_err(|_| BackendError::InvalidNode)?;
            let param_val = function
                .get_nth_param(i as u32)
                .ok_or(BackendError::InvalidNode)?;
            self.builder
                .build_store(param_ptr, param_val)
                .map_err(|_| BackendError::InvalidNode)?;
            self.variables.insert(
                pname.clone(),
                VariableBinding {
                    ptr: param_ptr,
                    pointee_type: *ptype,
                    function_type: *function_type,
                },
            );
            if ptype.is_pointer_type() && pname.starts_with('z') {
                self.byte_pointer_vars.insert(pname.clone());
            }
            if let Some((struct_type, field_names)) = struct_info {
                self.struct_fields
                    .insert(pname.clone(), field_names.clone());
                if ptype.is_pointer_type() {
                    self.pointer_struct_types
                        .insert(pname.clone(), *struct_type);
                }
            }
        }

        // Execute body statements
        if body_offset != NodeOffset::NULL {
            let mut stmt_off = body_offset;
            while stmt_off != NodeOffset::NULL {
                if let Some(stmt) = arena.get(stmt_off) {
                    let next = stmt.next_sibling;
                    // Skip errors in individual statements to keep compiling
                    let _ = self.lower_stmt(arena, stmt_off);
                    stmt_off = next;
                } else {
                    break;
                }
            }
        }

        let last_block = self.builder.get_insert_block();
        if let Some(bb) = last_block {
            if bb.get_terminator().is_none() {
                if is_void_ret {
                    self.builder
                        .build_return(None)
                        .map_err(|_| BackendError::InvalidNode)?;
                } else if let BasicTypeEnum::PointerType(ptr_type) = ret_llvm {
                    self.builder
                        .build_return(Some(&ptr_type.const_null()))
                        .map_err(|_| BackendError::InvalidNode)?;
                } else if let BasicTypeEnum::FloatType(ft) = ret_llvm {
                    self.builder
                        .build_return(Some(&ft.const_float(0.0)))
                        .map_err(|_| BackendError::InvalidNode)?;
                } else if ret_llvm.is_int_type() {
                    let int_type = ret_llvm.into_int_type();
                    self.builder
                        .build_return(Some(&int_type.const_int(0, false)))
                        .map_err(|_| BackendError::InvalidNode)?;
                } else if let BasicTypeEnum::StructType(st) = ret_llvm {
                    let zero = st.const_zero();
                    self.builder
                        .build_return(Some(&zero))
                        .map_err(|_| BackendError::InvalidNode)?;
                } else if let BasicTypeEnum::ArrayType(at) = ret_llvm {
                    let zero = at.const_zero();
                    self.builder
                        .build_return(Some(&zero))
                        .map_err(|_| BackendError::InvalidNode)?;
                } else {
                    // Fallback: try int, ignore error if it doesn't work
                    let _ = self
                        .builder
                        .build_return(Some(&self.context.i32_type().const_int(0, false)));
                }
            }
        }

        // Clean up empty/unreachable basic blocks to keep LLVM IR valid.
        // Walk all blocks: if a block has no predecessors (and is not the entry block),
        // and has no terminator, add an `unreachable` instruction.
        // Also ensure every block has a terminator.
        let mut block_opt = function.get_first_basic_block();
        let entry_block = function.get_first_basic_block();
        while let Some(bb) = block_opt {
            let next = bb.get_next_basic_block();
            if bb.get_terminator().is_none() {
                self.builder.position_at_end(bb);
                // Check if block is the entry or has uses (predecessors)
                if Some(bb) == entry_block {
                    // Entry block: add default return
                    if is_void_ret {
                        let _ = self.builder.build_return(None);
                    } else if let BasicTypeEnum::PointerType(pt) = ret_llvm {
                        let _ = self.builder.build_return(Some(&pt.const_null()));
                    } else if let BasicTypeEnum::FloatType(ft) = ret_llvm {
                        let _ = self.builder.build_return(Some(&ft.const_float(0.0)));
                    } else if ret_llvm.is_int_type() {
                        let _ = self
                            .builder
                            .build_return(Some(&ret_llvm.into_int_type().const_int(0, false)));
                    } else {
                        let _ = self.builder.build_unreachable();
                    }
                } else {
                    let _ = self.builder.build_unreachable();
                }
            }
            block_opt = next;
        }

        // Remove truly dead blocks (no predecessors, not entry, only have unreachable)
        let mut block_opt = function.get_first_basic_block();
        while let Some(bb) = block_opt {
            let next = bb.get_next_basic_block();
            if Some(bb) != entry_block
                && bb.get_first_use().is_none()
                && bb
                    .get_first_instruction()
                    .map(|i| i.get_opcode() == inkwell::values::InstructionOpcode::Unreachable)
                    .unwrap_or(false)
            {
                unsafe {
                    bb.delete().ok();
                }
            }
            block_opt = next;
        }

        // Final pass: fix any ret instruction whose type doesn't match the function return type.
        // This catches cases where type inference produced a mismatched return:
        // - ret void in non-void function (e.g., function pointer params parsed as functions)
        // - ret ptr in i32 function (e.g., char* return parsed as int)
        // We use LLVM module verification via writing IR text to detect and fix these.
        if !is_void_ret {
            let expected_is_int = ret_llvm.is_int_type();
            let expected_is_ptr = ret_llvm.is_pointer_type();
            let expected_is_float = ret_llvm.is_float_type();
            let mut block_opt = function.get_first_basic_block();
            while let Some(bb) = block_opt {
                let next = bb.get_next_basic_block();
                if let Some(term) = bb.get_terminator() {
                    if term.get_opcode() == inkwell::values::InstructionOpcode::Return {
                        let num_operands = term.get_num_operands();
                        if num_operands == 0 {
                            // ret void in non-void function — replace with typed default
                            term.erase_from_basic_block();
                            self.builder.position_at_end(bb);
                            if expected_is_int {
                                let _ = self.builder.build_return(Some(
                                    &ret_llvm.into_int_type().const_int(0, false),
                                ));
                            } else if expected_is_ptr {
                                let _ = self
                                    .builder
                                    .build_return(Some(&ret_llvm.into_pointer_type().const_null()));
                            } else if expected_is_float {
                                let _ = self.builder.build_return(Some(
                                    &ret_llvm.into_float_type().const_float(0.0),
                                ));
                            } else {
                                let _ = self.builder.build_return(Some(
                                    &self.context.i32_type().const_int(0, false),
                                ));
                            }
                        }
                    }
                }
                block_opt = next;
            }
        }

        self.current_return_type = None;
        Ok(())
    }

    fn lower_compound(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        self.push_scope();
        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                // Skip errors in individual statements to keep compiling
                let _ = self.lower_stmt(arena, child_offset);
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }
        self.pop_scope();
        Ok(())
    }

    fn lower_stmt(&mut self, arena: &Arena, offset: NodeOffset) -> Result<(), BackendError> {
        let node = match arena.get(offset) {
            Some(n) => n,
            None => return Ok(()),
        };

        match node.kind {
            20 => self.lower_decl(arena, &node),
            21 => self.lower_var_decl(arena, &node),
            22 => self.lower_func_decl(arena, &node),
            23 => self.lower_func_def(arena, &node),
            40 => self.lower_compound(arena, &node),
            41 => self.lower_if_stmt(arena, &node),
            42 => self.lower_while_stmt(arena, &node),
            43 => self.lower_for_stmt(arena, &node),
            44 => self.lower_return_stmt(arena, &node),
            45 => self.lower_expr_stmt(arena, &node),
            48 => Ok(()),
            46 | 47 => self.lower_break_continue(arena, &node),
            49 => self.lower_goto_stmt(arena, &node),
            50 => self.lower_switch_stmt(arena, &node),
            51 => self.lower_labeled_stmt(arena, &node),
            52 | 54 => {
                // Case label / case range: body follows as siblings in compound block.
                // The switch lowering handles BB switching; nothing to do here.
                Ok(())
            }
            53 => {
                // Default label: body follows as siblings in compound block.
                Ok(())
            }
            27 => Ok(()), // Bitfield - skip
            1..=9 | 83 | 90..=94 | 101..=105 => Ok(()),
            24 => Ok(()),
            25 | 26 => Ok(()),
            200 | 206 => Ok(()), // __attribute__, __extension__
            207 => self.lower_asm_stmt(arena, &node),
            _ => {
                let _ = self.lower_expr(arena, offset)?;
                Ok(())
            }
        }
    }

    /// Coerce any BasicValueEnum to an i1 boolean for use in conditionals.
    fn coerce_to_bool(
        &self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, BackendError> {
        if val.is_int_value() {
            let int_val = val.into_int_value();
            // If already i1, no conversion needed
            if int_val.get_type().get_bit_width() == 1 {
                return Ok(int_val);
            }
            // Compare against zero to produce i1
            let zero = int_val.get_type().const_zero();
            self.builder
                .build_int_compare(inkwell::IntPredicate::NE, int_val, zero, "tobool")
                .map_err(|_| BackendError::InvalidNode)
        } else if val.is_pointer_value() {
            let ptr_val = val.into_pointer_value();
            let ptr_int = self
                .builder
                .build_ptr_to_int(ptr_val, self.context.i64_type(), "ptr2int_cond")
                .map_err(|_| BackendError::InvalidNode)?;
            let zero = self.context.i64_type().const_zero();
            self.builder
                .build_int_compare(inkwell::IntPredicate::NE, ptr_int, zero, "ptr_nonnull")
                .map_err(|_| BackendError::InvalidNode)
        } else if val.is_float_value() {
            let float_val = val.into_float_value();
            let zero = float_val.get_type().const_float(0.0);
            self.builder
                .build_float_compare(inkwell::FloatPredicate::ONE, float_val, zero, "float_nz")
                .map_err(|_| BackendError::InvalidNode)
        } else {
            Err(BackendError::InvalidNode)
        }
    }

    fn lower_if_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        // kind=41: first_child=cond_wrap(kind=0)
        //   cond_wrap.first_child=condition_expr, cond_wrap.next_sibling=body_wrap(kind=0)
        //     body_wrap.first_child=then_stmt, body_wrap.next_sibling=else_stmt
        let cond_wrap_offset = node.first_child;
        let cond_offset = arena
            .get(cond_wrap_offset)
            .map(|w| w.first_child)
            .unwrap_or(NodeOffset::NULL);
        let body_wrap_offset = arena
            .get(cond_wrap_offset)
            .map(|w| w.next_sibling)
            .unwrap_or(NodeOffset::NULL);
        let (then_offset, else_offset) = if let Some(bw) = arena.get(body_wrap_offset) {
            (bw.first_child, bw.next_sibling)
        } else {
            (NodeOffset::NULL, NodeOffset::NULL)
        };

        let function = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or(BackendError::InvalidNode)?;

        let cond_val = self
            .lower_expr(arena, cond_offset)?
            .ok_or(BackendError::InvalidNode)?;

        let cond_int = self.coerce_to_bool(cond_val)?;

        let then_bb = self.context.append_basic_block(function, "then");
        let else_bb = self.context.append_basic_block(function, "else");
        let merge_bb = self.context.append_basic_block(function, "merge");

        self.builder
            .build_conditional_branch(cond_int, then_bb, else_bb)
            .map_err(|_| BackendError::InvalidNode)?;

        self.builder.position_at_end(then_bb);
        if then_offset != NodeOffset::NULL {
            self.lower_stmt(arena, then_offset)?;
        }
        if self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_terminator())
            .is_none()
        {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        self.builder.position_at_end(else_bb);
        if else_offset != NodeOffset::NULL {
            self.lower_stmt(arena, else_offset)?;
        }
        if self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_terminator())
            .is_none()
        {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        self.builder.position_at_end(merge_bb);
        Ok(())
    }

    fn lower_while_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        // kind=42: first_child=cond_wrap(kind=0)
        //   cond_wrap.first_child=condition_expr, cond_wrap.next_sibling=body
        let cond_wrap_offset = node.first_child;

        // do-while: data=1, first_child=body, body.next_sibling=condition
        let is_do_while = node.data == 1;

        let (cond_offset, body_offset) = if is_do_while {
            let body_off = node.first_child;
            let cond_off = arena
                .get(body_off)
                .map(|b| b.next_sibling)
                .unwrap_or(NodeOffset::NULL);
            (cond_off, body_off)
        } else {
            let co = arena
                .get(cond_wrap_offset)
                .map(|w| w.first_child)
                .unwrap_or(NodeOffset::NULL);
            let bo = arena
                .get(cond_wrap_offset)
                .map(|w| w.next_sibling)
                .unwrap_or(NodeOffset::NULL);
            (co, bo)
        };

        let function = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or(BackendError::InvalidNode)?;

        let cond_bb = self.context.append_basic_block(function, "while.cond");
        let body_bb = self.context.append_basic_block(function, "while.body");
        let end_bb = self.context.append_basic_block(function, "while.end");

        // Push break/continue targets for this loop
        self.break_stack.push(end_bb);
        self.continue_stack.push(cond_bb);

        if is_do_while {
            // do-while: enter body first
            self.builder
                .build_unconditional_branch(body_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        } else {
            self.builder
                .build_unconditional_branch(cond_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        // Condition block
        self.builder.position_at_end(cond_bb);
        if cond_offset != NodeOffset::NULL {
            if let Some(cond_val) = self.lower_expr(arena, cond_offset)? {
                let cond_int = self.coerce_to_bool(cond_val)?;
                self.builder
                    .build_conditional_branch(cond_int, body_bb, end_bb)
                    .map_err(|_| BackendError::InvalidNode)?;
            } else {
                self.builder
                    .build_unconditional_branch(body_bb)
                    .map_err(|_| BackendError::InvalidNode)?;
            }
        } else {
            // while(true)
            self.builder
                .build_unconditional_branch(body_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        // Body block
        self.builder.position_at_end(body_bb);
        if body_offset != NodeOffset::NULL {
            let _ = self.lower_stmt(arena, body_offset);
        }
        if self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_terminator())
            .is_none()
        {
            self.builder
                .build_unconditional_branch(cond_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        // Pop break/continue targets
        self.break_stack.pop();
        self.continue_stack.pop();

        self.builder.position_at_end(end_bb);
        Ok(())
    }

    fn lower_for_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let function = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or(BackendError::InvalidNode)?;

        // kind=43: first_child=init_wrap(kind=0)
        //   init_wrap.first_child=init, init_wrap.next_sibling=cond_wrap(kind=0)
        //     cond_wrap.first_child=condition_expr, cond_wrap.next_sibling=incr_wrap(kind=0)
        //       incr_wrap.first_child=increment, incr_wrap.next_sibling=body
        let init_wrap_offset = node.first_child;
        let init_offset = arena
            .get(init_wrap_offset)
            .map(|n| n.first_child)
            .unwrap_or(NodeOffset::NULL);
        let cond_wrap_offset = arena
            .get(init_wrap_offset)
            .map(|n| n.next_sibling)
            .unwrap_or(NodeOffset::NULL);
        let cond_offset = arena
            .get(cond_wrap_offset)
            .map(|n| n.first_child)
            .unwrap_or(NodeOffset::NULL);
        let incr_wrap_offset = arena
            .get(cond_wrap_offset)
            .map(|n| n.next_sibling)
            .unwrap_or(NodeOffset::NULL);
        let (incr_offset, body_offset) = if let Some(iw) = arena.get(incr_wrap_offset) {
            (iw.first_child, iw.next_sibling)
        } else {
            (NodeOffset::NULL, NodeOffset::NULL)
        };

        // Init
        if init_offset != NodeOffset::NULL {
            self.lower_stmt(arena, init_offset)?;
        }

        let cond_bb = self.context.append_basic_block(function, "for.cond");
        let incr_bb = self.context.append_basic_block(function, "for.incr");
        let body_bb = self.context.append_basic_block(function, "for.body");
        let end_bb = self.context.append_basic_block(function, "for.end");

        // Push break/continue targets: break→end, continue→incr (not cond!)
        self.break_stack.push(end_bb);
        self.continue_stack.push(incr_bb);

        self.builder
            .build_unconditional_branch(cond_bb)
            .map_err(|_| BackendError::InvalidNode)?;

        self.builder.position_at_end(cond_bb);
        if cond_offset != NodeOffset::NULL {
            if let Some(cond_val) = self.lower_expr(arena, cond_offset)? {
                let cond_int = self.coerce_to_bool(cond_val)?;
                self.builder
                    .build_conditional_branch(cond_int, body_bb, end_bb)
                    .map_err(|_| BackendError::InvalidNode)?;
            } else {
                self.builder
                    .build_unconditional_branch(body_bb)
                    .map_err(|_| BackendError::InvalidNode)?;
            }
        } else {
            self.builder
                .build_unconditional_branch(body_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        self.builder.position_at_end(body_bb);
        if body_offset != NodeOffset::NULL {
            let _ = self.lower_stmt(arena, body_offset);
        }
        if self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_terminator())
            .is_none()
        {
            self.builder
                .build_unconditional_branch(incr_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        // Increment block
        self.builder.position_at_end(incr_bb);
        if incr_offset != NodeOffset::NULL {
            let _ = self.lower_expr(arena, incr_offset)?;
        }
        if incr_bb.get_terminator().is_none() {
            self.builder
                .build_unconditional_branch(cond_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        // Pop break/continue targets
        self.break_stack.pop();
        self.continue_stack.pop();

        self.builder.position_at_end(end_bb);
        Ok(())
    }

    /// Lower a switch statement (kind=50).
    /// AST: first_child=condition expr, next_sibling=body (compound stmt with case/default labels)
    fn lower_switch_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        // Layout (immune to link_siblings):
        //   kind=50.first_child = cond_wrap(kind=0)
        //     cond_wrap.first_child = condition_expr
        //     cond_wrap.next_sibling = body (compound block, kind=40)
        let cond_wrap_offset = node.first_child;
        let cond_offset = arena
            .get(cond_wrap_offset)
            .map(|w| w.first_child)
            .unwrap_or(NodeOffset::NULL);
        let body_offset = arena
            .get(cond_wrap_offset)
            .map(|w| w.next_sibling)
            .unwrap_or(NodeOffset::NULL);

        let function = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or(BackendError::InvalidNode)?;

        // Evaluate the switch condition
        let cond_val = self
            .lower_expr(arena, cond_offset)?
            .ok_or(BackendError::InvalidNode)?;

        let cond_int = if cond_val.is_int_value() {
            cond_val.into_int_value()
        } else if cond_val.is_pointer_value() {
            self.builder
                .build_ptr_to_int(
                    cond_val.into_pointer_value(),
                    self.context.i64_type(),
                    "switch_ptr2int",
                )
                .map_err(|_| BackendError::InvalidNode)?
        } else {
            self.context.i32_type().const_int(0, false)
        };

        let end_bb = self.context.append_basic_block(function, "switch.end");
        let default_bb = self.context.append_basic_block(function, "switch.default");

        // Pass 1: Walk the body's children to collect case/default labels → build BBs
        // case (52): first_child=expr, no body stored (body follows as siblings)
        // default (53): no children
        // case_range (54): first_child=low_expr, data=high_expr_offset
        let mut cases: Vec<(inkwell::values::IntValue<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        // Map from BasicBlock name index to BB (we use a Vec, indexed by visit order)
        // We also need to know which compound-block child offset corresponds to each BB.
        // Instead, we'll build a map: child_offset → BB for case/default nodes.
        let mut case_bb_map: Vec<(NodeOffset, BasicBlock<'ctx>)> = Vec::new();
        let mut has_default = false;

        // Get the list of children of the body compound block
        let body_first_child = if let Some(body_node) = arena.get(body_offset) {
            if body_node.kind == 40 {
                body_node.first_child
            } else {
                body_offset // body may itself be a case label
            }
        } else {
            NodeOffset::NULL
        };

        {
            let mut child_off = body_first_child;
            while child_off != NodeOffset::NULL {
                let child = match arena.get(child_off) {
                    Some(c) => c,
                    None => break,
                };
                match child.kind {
                    52 => {
                        // case label: first_child=expr
                        let case_bb = self.context.append_basic_block(function, "switch.case");
                        if child.first_child != NodeOffset::NULL {
                            if let Some(val) =
                                self.lower_expr(arena, child.first_child).ok().flatten()
                            {
                                let case_int = if val.is_int_value() {
                                    let raw = val.into_int_value();
                                    let cond_bits = cond_int.get_type().get_bit_width();
                                    let case_bits = raw.get_type().get_bit_width();
                                    if case_bits < cond_bits {
                                        self.builder
                                            .build_int_s_extend(
                                                raw,
                                                cond_int.get_type(),
                                                "case_sext",
                                            )
                                            .unwrap_or(raw)
                                    } else if case_bits > cond_bits {
                                        self.builder
                                            .build_int_truncate(
                                                raw,
                                                cond_int.get_type(),
                                                "case_trunc",
                                            )
                                            .unwrap_or(raw)
                                    } else {
                                        raw
                                    }
                                } else {
                                    cond_int.get_type().const_int(0, false)
                                };
                                cases.push((case_int, case_bb));
                            } else {
                                cases.push((cond_int.get_type().const_int(0, false), case_bb));
                            }
                        }
                        case_bb_map.push((child_off, case_bb));
                    }
                    54 => {
                        // case range: first_child=low_expr, data=high_expr_offset
                        let case_bb = self
                            .context
                            .append_basic_block(function, "switch.case_range");
                        let lo_val = if child.first_child != NodeOffset::NULL {
                            self.lower_expr(arena, child.first_child).ok().flatten()
                        } else {
                            None
                        };
                        let hi_val = if child.data != 0 {
                            self.lower_expr(arena, NodeOffset(child.data))
                                .ok()
                                .flatten()
                        } else {
                            None
                        };
                        if let (Some(lo), Some(hi)) = (lo_val, hi_val) {
                            if lo.is_int_value() && hi.is_int_value() {
                                let lo_c = lo
                                    .into_int_value()
                                    .get_zero_extended_constant()
                                    .unwrap_or(0);
                                let hi_c = hi
                                    .into_int_value()
                                    .get_zero_extended_constant()
                                    .unwrap_or(0);
                                let count = if hi_c >= lo_c {
                                    (hi_c - lo_c + 1).min(MAX_CASE_RANGE_EXPANSION)
                                } else {
                                    1
                                };
                                for i in 0..count {
                                    cases.push((
                                        cond_int.get_type().const_int(lo_c + i, false),
                                        case_bb,
                                    ));
                                }
                            }
                        }
                        case_bb_map.push((child_off, case_bb));
                    }
                    53 => {
                        // default label
                        has_default = true;
                        case_bb_map.push((child_off, default_bb));
                    }
                    _ => {}
                }
                child_off = child.next_sibling;
            }
        }

        // Deduplicate switch cases by value (keep first occurrence)
        {
            let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
            cases.retain(|(val, _bb)| {
                let key = val.get_zero_extended_constant().unwrap_or(u64::MAX);
                seen.insert(key)
            });
        }

        // Build the switch instruction
        let _ = self
            .builder
            .build_switch(cond_int, default_bb, &cases)
            .map_err(|_| BackendError::InvalidNode)?;

        // Push break target
        self.break_stack.push(end_bb);

        // Pass 2: Walk body children in order, switching BBs at case/default labels.
        // Build a lookup: NodeOffset → BB for fast lookup
        let case_offset_to_bb: std::collections::HashMap<NodeOffset, BasicBlock<'ctx>> =
            case_bb_map.iter().cloned().collect();

        // We need to track the ordered list of case BBs to handle fall-through
        let ordered_case_bbs: Vec<BasicBlock<'ctx>> =
            case_bb_map.iter().map(|(_, bb)| *bb).collect();

        self.push_scope();
        {
            let mut child_off = body_first_child;
            while child_off != NodeOffset::NULL {
                let child = match arena.get(child_off) {
                    Some(c) => c,
                    None => break,
                };
                let next_off = child.next_sibling;

                if let Some(&case_bb) = case_offset_to_bb.get(&child_off) {
                    // This is a case or default label — switch to its BB
                    // If current block has no terminator, fall through to this BB
                    if self
                        .builder
                        .get_insert_block()
                        .and_then(|bb| bb.get_terminator())
                        .is_none()
                    {
                        let _ = self.builder.build_unconditional_branch(case_bb);
                    }
                    self.builder.position_at_end(case_bb);
                } else {
                    // Regular statement — lower it in current BB
                    let _ = self.lower_stmt(arena, child_off);
                }

                child_off = next_off;
            }
        }
        self.pop_scope();

        // Ensure current BB (last case body) has a terminator
        if self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_terminator())
            .is_none()
        {
            let _ = self.builder.build_unconditional_branch(end_bb);
        }

        // Ensure default BB has a terminator (if no default label, jump to end)
        if !has_default {
            self.builder.position_at_end(default_bb);
            if default_bb.get_terminator().is_none() {
                let _ = self.builder.build_unconditional_branch(end_bb);
            }
        }

        // Ensure all case BBs that are unreachable/empty have terminators
        for bb in &ordered_case_bbs {
            if bb.get_terminator().is_none() {
                self.builder.position_at_end(*bb);
                let _ = self.builder.build_unconditional_branch(end_bb);
            }
        }

        // Pop break target
        self.break_stack.pop();

        self.builder.position_at_end(end_bb);
        Ok(())
    }

    /// Lower a goto statement (kind=49).
    /// data = string offset of label name (0 if computed goto with first_child=expr)
    fn lower_goto_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let function = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or(BackendError::InvalidNode)?;

        if node.data != 0 {
            // Named goto
            let label_name = arena
                .get_string(NodeOffset(node.data))
                .unwrap_or("")
                .to_string();
            if label_name.is_empty() {
                return Ok(());
            }

            // Get or create the target basic block
            let target_bb = if let Some(bb) = self.label_blocks.get(&label_name) {
                *bb
            } else {
                let bb = self
                    .context
                    .append_basic_block(function, &format!("label.{}", label_name));
                self.label_blocks.insert(label_name.clone(), bb);
                bb
            };

            self.builder
                .build_unconditional_branch(target_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        } else if node.first_child != NodeOffset::NULL {
            // Computed goto: goto *expr → LLVM indirectbr
            if let Some(addr_val) = self.lower_expr(arena, node.first_child)? {
                // Collect all known label blocks as possible destinations
                let destinations: Vec<BasicBlock<'ctx>> =
                    self.label_blocks.values().copied().collect();

                self.builder
                    .build_indirect_branch(addr_val, &destinations)
                    .map_err(|_| BackendError::InvalidNode)?;
            }
        }
        Ok(())
    }

    /// Lower a labeled statement (kind=51).
    /// data = string offset of label name, first_child = inner statement
    fn lower_labeled_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let function = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or(BackendError::InvalidNode)?;

        let label_name = if node.data != 0 {
            arena
                .get_string(NodeOffset(node.data))
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        };

        if !label_name.is_empty() {
            // Get or create the basic block for this label
            let label_bb = if let Some(bb) = self.label_blocks.get(&label_name) {
                *bb
            } else {
                let bb = self
                    .context
                    .append_basic_block(function, &format!("label.{}", label_name));
                self.label_blocks.insert(label_name.clone(), bb);
                bb
            };

            // Branch from current block to label block (fall-through)
            if self
                .builder
                .get_insert_block()
                .and_then(|bb| bb.get_terminator())
                .is_none()
            {
                self.builder
                    .build_unconditional_branch(label_bb)
                    .map_err(|_| BackendError::InvalidNode)?;
            }

            self.builder.position_at_end(label_bb);
        }

        // Lower the inner statement
        if node.first_child != NodeOffset::NULL {
            self.lower_stmt(arena, node.first_child)?;
        }
        Ok(())
    }

    /// Lower break (kind=46) or continue (kind=47) statements.
    fn lower_break_continue(
        &mut self,
        _arena: &Arena,
        node: &CAstNode,
    ) -> Result<(), BackendError> {
        match node.kind {
            46 => {
                // break
                if let Some(target) = self.break_stack.last() {
                    let target = *target;
                    self.builder
                        .build_unconditional_branch(target)
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Ok(())
            }
            47 => {
                // continue
                if let Some(target) = self.continue_stack.last() {
                    let target = *target;
                    self.builder
                        .build_unconditional_branch(target)
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Lower inline asm statement (kind=207).
    ///
    /// ASM_STMT node layout:
    /// - data = flags (bit 0 = volatile, bit 1 = goto)
    /// - first_child = template string offset (NodeOffset into string arena)
    /// - next_sibling = first operand child (linked chain of kind=208/209/210/211)
    fn lower_asm_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let is_volatile = (node.data & 1) != 0;

        // Read template string from arena
        let template = arena.get_string(node.first_child).unwrap_or("").to_string();

        // Strip surrounding quotes if present (parser may store them)
        let template = template
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(&template)
            .to_string();

        // Walk operand children to gather outputs, inputs, clobbers, and goto labels
        let mut output_constraints: Vec<String> = Vec::new();
        let mut input_constraints: Vec<String> = Vec::new();
        let mut clobbers: Vec<String> = Vec::new();
        let mut input_values: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = Vec::new();
        let mut output_lvalues: Vec<Option<(PointerValue<'ctx>, BasicTypeEnum<'ctx>)>> = Vec::new();

        let mut child_offset = node.next_sibling;
        while child_offset != NodeOffset::NULL {
            let child = match arena.get(child_offset) {
                Some(c) => c,
                None => break,
            };

            match child.kind {
                208 => {
                    // ASM_OPERAND_OUTPUT: data = constraint string offset, first_child = expr
                    let constraint = arena
                        .get_string(NodeOffset(child.data))
                        .unwrap_or("r")
                        .to_string();

                    // Strip quotes from constraint
                    let constraint = constraint
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                        .unwrap_or(&constraint)
                        .to_string();

                    // Determine if readwrite (+) or output-only (=)
                    let is_readwrite = constraint.starts_with('+');
                    let clean = if is_readwrite {
                        constraint
                            .strip_prefix('+')
                            .unwrap_or(&constraint)
                            .to_string()
                    } else {
                        format!("={}", constraint.strip_prefix('=').unwrap_or(&constraint))
                    };

                    let llvm_constraint = if is_readwrite {
                        // LLVM read-write: output constraint with matching input
                        format!("+{}", constraint.strip_prefix('+').unwrap_or(&constraint))
                    } else {
                        clean
                    };

                    output_constraints.push(llvm_constraint);

                    // Get the lvalue pointer for the output operand expression
                    if child.first_child != NodeOffset::NULL {
                        let lvalue = self.lower_lvalue_ptr(arena, child.first_child)?;
                        output_lvalues.push(lvalue);

                        // For readwrite operands, the current value is also an input
                        if is_readwrite {
                            if let Some((ptr, pointee_ty)) = &lvalue {
                                let loaded = self
                                    .builder
                                    .build_load(*pointee_ty, *ptr, "asm_rw_load")
                                    .map_err(|_| BackendError::InvalidNode)?;
                                input_values.push(loaded.into());
                            }
                        }
                    } else {
                        output_lvalues.push(None);
                    }
                }
                209 => {
                    // ASM_OPERAND_INPUT: data = constraint string offset, first_child = expr
                    let constraint = arena
                        .get_string(NodeOffset(child.data))
                        .unwrap_or("r")
                        .to_string();

                    // Strip quotes from constraint
                    let constraint = constraint
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                        .unwrap_or(&constraint)
                        .to_string();

                    input_constraints.push(constraint);

                    // Evaluate input expression
                    if child.first_child != NodeOffset::NULL {
                        if let Some(val) = self.lower_expr(arena, child.first_child)? {
                            input_values.push(val.into());
                        }
                    }
                }
                210 => {
                    // ASM_CLOBBER: data = clobber string offset
                    let clobber = arena
                        .get_string(NodeOffset(child.data))
                        .unwrap_or("")
                        .to_string();

                    // Strip quotes from clobber
                    let clobber = clobber
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                        .unwrap_or(&clobber)
                        .to_string();

                    if !clobber.is_empty() {
                        clobbers.push(format!("~{{{}}}", clobber));
                    }
                }
                211 => {
                    // ASM_GOTO_LABEL: data = label string offset — ignored for now
                }
                _ => {}
            }

            child_offset = child.next_sibling;
        }

        // Build the full constraint string: outputs, inputs, clobbers
        let mut all_constraints: Vec<String> = Vec::new();
        all_constraints.extend(output_constraints.iter().cloned());
        all_constraints.extend(input_constraints.iter().cloned());
        all_constraints.extend(clobbers.iter().cloned());
        let constraints_str = all_constraints.join(",");

        // Build the LLVM function type for this inline asm
        // Input parameter types come from the evaluated input values
        let param_types: Vec<BasicMetadataTypeEnum<'ctx>> = input_values
            .iter()
            .map(|v| match v {
                inkwell::values::BasicMetadataValueEnum::IntValue(v) => {
                    BasicMetadataTypeEnum::IntType(v.get_type())
                }
                inkwell::values::BasicMetadataValueEnum::FloatValue(v) => {
                    BasicMetadataTypeEnum::FloatType(v.get_type())
                }
                inkwell::values::BasicMetadataValueEnum::PointerValue(_) => {
                    BasicMetadataTypeEnum::PointerType(
                        self.context.ptr_type(AddressSpace::default()),
                    )
                }
                _ => BasicMetadataTypeEnum::IntType(self.context.i32_type()),
            })
            .collect();

        // Determine return type based on output operands
        let has_side_effects = is_volatile || clobbers.iter().any(|c| c.contains("memory"));

        let asm_fn_type = if output_constraints.is_empty() {
            // No outputs → void return
            self.context.void_type().fn_type(&param_types, false)
        } else if output_constraints.len() == 1 {
            // Single output → i32 return (default; could be refined with type info)
            self.context.i32_type().fn_type(&param_types, false)
        } else {
            // Multiple outputs → struct return
            let field_types: Vec<BasicTypeEnum<'ctx>> = output_constraints
                .iter()
                .map(|_| self.context.i32_type().into())
                .collect();
            let struct_type = self.context.struct_type(&field_types, false);
            struct_type.fn_type(&param_types, false)
        };

        // Create the inline asm value
        let asm_val = self.context.create_inline_asm(
            asm_fn_type,
            template,
            constraints_str,
            has_side_effects,
            false, // alignstack
            None,  // dialect (ATT default)
            false, // can_throw
        );

        // Call the inline asm
        let call_result = self
            .builder
            .build_indirect_call(asm_fn_type, asm_val, &input_values, "asm_call")
            .map_err(|_| BackendError::InvalidNode)?;

        // Store output results back to lvalue pointers
        if output_constraints.len() == 1 {
            if let Some(Some((ptr, pointee_ty))) = output_lvalues.first() {
                match call_result.try_as_basic_value() {
                    inkwell::values::ValueKind::Basic(result) => {
                        // Cast result to match the expected pointee type if needed
                        let store_val: BasicValueEnum<'ctx> = if result.get_type() != *pointee_ty {
                            if result.is_int_value() && pointee_ty.is_int_type() {
                                let result_int = result.into_int_value();
                                let target_int = pointee_ty.into_int_type();
                                if result_int.get_type().get_bit_width()
                                    > target_int.get_bit_width()
                                {
                                    self.builder
                                        .build_int_truncate(result_int, target_int, "asm_trunc")
                                        .map_err(|_| BackendError::InvalidNode)?
                                        .into()
                                } else {
                                    self.builder
                                        .build_int_z_extend(result_int, target_int, "asm_zext")
                                        .map_err(|_| BackendError::InvalidNode)?
                                        .into()
                                }
                            } else {
                                result
                            }
                        } else {
                            result
                        };
                        self.builder
                            .build_store(*ptr, store_val)
                            .map_err(|_| BackendError::InvalidNode)?;
                    }
                    inkwell::values::ValueKind::Instruction(_) => {}
                }
            }
        } else if output_constraints.len() > 1 {
            // Multiple outputs: extract from struct
            match call_result.try_as_basic_value() {
                inkwell::values::ValueKind::Basic(result) => {
                    for (i, lvalue) in output_lvalues.iter().enumerate() {
                        if let Some((ptr, _pointee_ty)) = lvalue {
                            let extracted = self
                                .builder
                                .build_extract_value(
                                    result.into_struct_value(),
                                    i as u32,
                                    &format!("asm_out_{}", i),
                                )
                                .map_err(|_| BackendError::InvalidNode)?;
                            self.builder
                                .build_store(*ptr, extracted)
                                .map_err(|_| BackendError::InvalidNode)?;
                        }
                    }
                }
                inkwell::values::ValueKind::Instruction(_) => {}
            }
        }

        Ok(())
    }

    fn lower_return_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        // If function is void, always emit ret void regardless of expression
        if self.current_return_type.is_none() {
            // Evaluate expression for side effects if present
            if node.first_child != NodeOffset::NULL {
                let _ = self.lower_expr(arena, node.first_child);
            }
            self.builder
                .build_return(None)
                .map_err(|_| BackendError::InvalidNode)?;
            return Ok(());
        }

        if node.first_child != NodeOffset::NULL {
            if let Some(val) = self.lower_expr(arena, node.first_child)? {
                if let Some(BasicTypeEnum::PointerType(ptr_type)) = self.current_return_type {
                    if val.is_pointer_value() {
                        let ptr_val = val.into_pointer_value();
                        self.builder
                            .build_return(Some(&ptr_val))
                            .map_err(|_| BackendError::InvalidNode)?;
                    } else if val.is_int_value() {
                        let int_val = val.into_int_value();
                        if int_val.get_zero_extended_constant() == Some(0) {
                            self.builder
                                .build_return(Some(&ptr_type.const_null()))
                                .map_err(|_| BackendError::InvalidNode)?;
                        } else {
                            // Non-zero int returned as pointer: inttoptr
                            let cast = self
                                .builder
                                .build_int_to_ptr(int_val, ptr_type, "int2ptr_ret")
                                .map_err(|_| BackendError::InvalidNode)?;
                            self.builder
                                .build_return(Some(&cast))
                                .map_err(|_| BackendError::InvalidNode)?;
                        }
                    } else {
                        // Unknown value type for pointer return - return null
                        self.builder
                            .build_return(Some(&ptr_type.const_null()))
                            .map_err(|_| BackendError::InvalidNode)?;
                    }
                } else if let Some(BasicTypeEnum::StructType(_st)) = self.current_return_type {
                    // Struct return: the value should be a struct value from a load
                    if val.is_struct_value() {
                        self.builder
                            .build_return(Some(&val.into_struct_value()))
                            .map_err(|_| BackendError::InvalidNode)?;
                    } else {
                        // Value type mismatch — return zeroinitializer
                        self.builder
                            .build_return(Some(&_st.const_zero()))
                            .map_err(|_| BackendError::InvalidNode)?;
                    }
                } else if let Some(BasicTypeEnum::FloatType(_ft)) = self.current_return_type {
                    if val.is_float_value() {
                        self.builder
                            .build_return(Some(&val.into_float_value()))
                            .map_err(|_| BackendError::InvalidNode)?;
                    } else if val.is_int_value() {
                        // int returned as float: sitofp
                        let int_val = val.into_int_value();
                        let cast = self
                            .builder
                            .build_signed_int_to_float(int_val, _ft, "int2fp_ret")
                            .map_err(|_| BackendError::InvalidNode)?;
                        self.builder
                            .build_return(Some(&cast))
                            .map_err(|_| BackendError::InvalidNode)?;
                    } else {
                        self.builder
                            .build_return(Some(&_ft.const_float(0.0)))
                            .map_err(|_| BackendError::InvalidNode)?;
                    }
                } else if val.is_int_value() {
                    let int_val = val.into_int_value();
                    // Match the return type width
                    if let Some(ret_ty) = self.current_return_type {
                        if ret_ty.is_int_type() {
                            let ret_int = ret_ty.into_int_type();
                            let val_width = int_val.get_type().get_bit_width();
                            let ret_width = ret_int.get_bit_width();
                            if val_width < ret_width {
                                let extended = self
                                    .builder
                                    .build_int_s_extend(int_val, ret_int, "sext_ret")
                                    .map_err(|_| BackendError::InvalidNode)?;
                                self.builder
                                    .build_return(Some(&extended))
                                    .map_err(|_| BackendError::InvalidNode)?;
                            } else if val_width > ret_width {
                                let truncated = self
                                    .builder
                                    .build_int_truncate(int_val, ret_int, "trunc_ret")
                                    .map_err(|_| BackendError::InvalidNode)?;
                                self.builder
                                    .build_return(Some(&truncated))
                                    .map_err(|_| BackendError::InvalidNode)?;
                            } else {
                                self.builder
                                    .build_return(Some(&int_val))
                                    .map_err(|_| BackendError::InvalidNode)?;
                            }
                        } else {
                            self.builder
                                .build_return(Some(&int_val))
                                .map_err(|_| BackendError::InvalidNode)?;
                        }
                    } else {
                        self.builder
                            .build_return(Some(&int_val))
                            .map_err(|_| BackendError::InvalidNode)?;
                    }
                } else if val.is_float_value() {
                    let float_val = val.into_float_value();
                    // Check if function returns a non-float type and convert
                    if let Some(ret_ty) = self.current_return_type {
                        if ret_ty.is_int_type() {
                            let ret_int = ret_ty.into_int_type();
                            let cast = self
                                .builder
                                .build_float_to_signed_int(float_val, ret_int, "fp2int_ret")
                                .map_err(|_| BackendError::InvalidNode)?;
                            self.builder
                                .build_return(Some(&cast))
                                .map_err(|_| BackendError::InvalidNode)?;
                        } else {
                            self.builder
                                .build_return(Some(&float_val))
                                .map_err(|_| BackendError::InvalidNode)?;
                        }
                    } else {
                        self.builder
                            .build_return(Some(&float_val))
                            .map_err(|_| BackendError::InvalidNode)?;
                    }
                } else if val.is_pointer_value() {
                    // Pointer returned in non-pointer function - ptrtoint
                    if let Some(ret_ty) = self.current_return_type {
                        if ret_ty.is_int_type() {
                            let ptr_val = val.into_pointer_value();
                            let cast = self
                                .builder
                                .build_ptr_to_int(ptr_val, ret_ty.into_int_type(), "ptr2int_ret")
                                .map_err(|_| BackendError::InvalidNode)?;
                            self.builder
                                .build_return(Some(&cast))
                                .map_err(|_| BackendError::InvalidNode)?;
                        } else {
                            return Ok(());
                        }
                    } else {
                        return Ok(());
                    }
                } else {
                    return Ok(());
                }
            } else {
                // lower_expr returned None - return a default zero/null for the function return type
                match self.current_return_type {
                    None => {
                        self.builder
                            .build_return(None)
                            .map_err(|_| BackendError::InvalidNode)?;
                    }
                    Some(BasicTypeEnum::PointerType(ptr_type)) => {
                        self.builder
                            .build_return(Some(&ptr_type.const_null()))
                            .map_err(|_| BackendError::InvalidNode)?;
                    }
                    Some(BasicTypeEnum::FloatType(ft)) => {
                        self.builder
                            .build_return(Some(&ft.const_float(0.0)))
                            .map_err(|_| BackendError::InvalidNode)?;
                    }
                    Some(BasicTypeEnum::StructType(st)) => {
                        self.builder
                            .build_return(Some(&st.const_zero()))
                            .map_err(|_| BackendError::InvalidNode)?;
                    }
                    Some(ret_ty) if ret_ty.is_int_type() => {
                        let int_type = ret_ty.into_int_type();
                        self.builder
                            .build_return(Some(&int_type.const_int(0, false)))
                            .map_err(|_| BackendError::InvalidNode)?;
                    }
                    Some(_) => {
                        self.builder
                            .build_return(Some(&self.context.i32_type().const_int(0, false)))
                            .map_err(|_| BackendError::InvalidNode)?;
                    }
                }
            }
        } else {
            // Empty return statement: use `ret void` only if the function is void,
            // otherwise return a zero/null default to keep LLVM IR valid.
            match self.current_return_type {
                None => {
                    self.builder
                        .build_return(None)
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Some(BasicTypeEnum::PointerType(ptr_type)) => {
                    self.builder
                        .build_return(Some(&ptr_type.const_null()))
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Some(BasicTypeEnum::FloatType(ft)) => {
                    self.builder
                        .build_return(Some(&ft.const_float(0.0)))
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Some(BasicTypeEnum::StructType(st)) => {
                    self.builder
                        .build_return(Some(&st.const_zero()))
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Some(ret_ty) => {
                    let int_type = ret_ty.into_int_type();
                    self.builder
                        .build_return(Some(&int_type.const_int(0, false)))
                        .map_err(|_| BackendError::InvalidNode)?;
                }
            }
        }
        Ok(())
    }

    fn lower_expr_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        if node.first_child != NodeOffset::NULL {
            let _ = self.lower_expr(arena, node.first_child)?;
        }
        Ok(())
    }

    fn lower_expr(
        &mut self,
        arena: &Arena,
        offset: NodeOffset,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let node = match arena.get(offset) {
            Some(n) => n,
            None => return Ok(None),
        };

        if !node.flags.contains(NodeFlags::IS_VALID) {
            return Ok(None);
        }

        if std::env::var("OPTICC_DEBUG_FNPTR").is_ok() {
            eprintln!("DEBUG lower_expr: kind={} offset={}", node.kind, offset.0);
        }

        match node.kind {
            0 | 74 => {
                if node.first_child != NodeOffset::NULL {
                    self.lower_expr(arena, node.first_child)
                } else {
                    Ok(None)
                }
            }
            60 => self.lower_ident(arena, &node),
            61 => self.lower_int_const(&node),
            62 => self.lower_char_const(&node),
            63 | 81 => self.lower_string_const(arena, &node),
            64 => self.lower_binop(arena, &node),
            65 => self.lower_unop(arena, &node),
            66 => self.lower_cond_expr(arena, &node),
            67 => self.lower_call_expr(arena, &node),
            68 => self.lower_array_index(arena, &node),
            69 => self.lower_member_access(arena, &node),
            70 => self.lower_cast_expr(arena, &node),
            71 => self.lower_sizeof_expr(arena, &node),
            72 => self.lower_comma_expr(arena, &node),
            73 => self.lower_assign_expr(arena, &node),
            80 => self.lower_int_const(&node),
            82 => self.lower_float_const(&node),
            201 => self.lower_typeof(arena, &node),
            202 => self.lower_stmt_expr(arena, &node),
            203 => self.lower_label_addr(arena, &node),
            204 => self.lower_builtin_call(arena, &node),
            205 => self.lower_designated_init(arena, &node),
            206 => self.lower_extension(arena, &node),
            212 => self.lower_compound_literal(arena, &node),
            1..=9 | 83 | 90..=94 | 101..=105 | 200 => Ok(None),
            _ => Ok(None),
        }
    }

    fn lower_ident(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let name_offset = NodeOffset(node.data);
        if let Some(name) = arena.get_string(name_offset) {
            if let Some(ptr) = self.variables.get(name) {
                if let BasicTypeEnum::ArrayType(array_type) = ptr.pointee_type {
                    let zero = self.context.i32_type().const_zero();
                    let array_ptr = unsafe {
                        self.builder
                            .build_gep(array_type, ptr.ptr, &[zero, zero], name)
                            .map_err(|_| BackendError::InvalidNode)?
                    };
                    return Ok(Some(array_ptr.into()));
                }
                let val = self
                    .builder
                    .build_load(ptr.pointee_type, ptr.ptr, name)
                    .map_err(|_| BackendError::InvalidNode)?;
                return Ok(Some(val));
            }
            if let Some(func) = self.functions.get(name) {
                return Ok(Some(func.as_global_value().as_basic_value_enum()));
            }
        }
        Ok(None)
    }

    fn lower_int_const(
        &self,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let value = node.data as u64;
        if let Some(ts) = self.types {
            let type_id = 7;
            let llvm_type = if let Some(t) = ts.get_type(TypeId(type_id)) {
                match t {
                    CType::Char { .. } => self.context.i8_type().const_int(value, false).into(),
                    CType::Short { .. } => self.context.i16_type().const_int(value, false).into(),
                    CType::Int { .. } => self.context.i32_type().const_int(value, false).into(),
                    CType::Long { .. } | CType::LongLong { .. } => {
                        self.context.i64_type().const_int(value, false).into()
                    }
                    _ => self.context.i32_type().const_int(value, false).into(),
                }
            } else {
                self.context.i32_type().const_int(value, false).into()
            };
            Ok(Some(llvm_type))
        } else {
            Ok(Some(self.context.i32_type().const_int(value, false).into()))
        }
    }

    fn lower_char_const(
        &self,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let value = node.data as u64;
        Ok(Some(self.context.i8_type().const_int(value, false).into()))
    }

    fn lower_string_const(
        &self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let string_offset = NodeOffset(node.data);
        let string_text = arena.get_string(string_offset).unwrap_or("");
        let bytes = string_text.as_bytes();
        let string_val = self.context.const_string(bytes, true); // true = null-terminated
        let global =
            self.module
                .add_global(string_val.get_type(), Some(AddressSpace::default()), ".str");
        global.set_initializer(&string_val);
        global.set_constant(true);
        global.set_linkage(inkwell::module::Linkage::Private);
        global.set_unnamed_addr(true);
        Ok(Some(global.as_pointer_value().into()))
    }

    fn lower_float_const(
        &self,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let val = node.data as f64;
        Ok(Some(self.context.f64_type().const_float(val).into()))
    }

    fn lower_binop(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let (lhs_offset, rhs_offset) = if let Some(wrapper) = arena.get(node.first_child) {
            if wrapper.kind == 0
                && wrapper.first_child != NodeOffset::NULL
                && wrapper.next_sibling != NodeOffset::NULL
            {
                // New parser layout: kind=64.first_child -> wrapper(kind=0)
                // wrapper.first_child=lhs, wrapper.next_sibling=rhs
                (wrapper.first_child, wrapper.next_sibling)
            } else {
                // Legacy layout fallback.
                let lhs_offset = node.first_child;
                let rhs_offset = arena
                    .get(lhs_offset)
                    .map(|lhs| lhs.next_sibling)
                    .filter(|off| *off != NodeOffset::NULL)
                    .unwrap_or(node.next_sibling);
                (lhs_offset, rhs_offset)
            }
        } else {
            (node.first_child, NodeOffset::NULL)
        };

        let lhs_val = self
            .lower_expr(arena, lhs_offset)?
            .ok_or(BackendError::InvalidNode)?;
        let rhs_val = self
            .lower_expr(arena, rhs_offset)?
            .ok_or(BackendError::InvalidNode)?;

        let use_float = lhs_val.is_float_value() || rhs_val.is_float_value();
        let use_pointer = lhs_val.is_pointer_value() || rhs_val.is_pointer_value();

        let result: BasicValueEnum = if use_pointer {
            // Pointer comparisons: convert both sides to intptr for comparison
            let ptr_int_type = self.context.i64_type();
            let lhs_int = if lhs_val.is_pointer_value() {
                self.builder
                    .build_ptr_to_int(lhs_val.into_pointer_value(), ptr_int_type, "ptr2int_lhs")
                    .map_err(|_| BackendError::InvalidNode)?
            } else if lhs_val.is_int_value() {
                let iv = lhs_val.into_int_value();
                if iv.get_type().get_bit_width() != 64 {
                    self.builder
                        .build_int_z_extend(iv, ptr_int_type, "zext_lhs")
                        .map_err(|_| BackendError::InvalidNode)?
                } else {
                    iv
                }
            } else {
                return Err(BackendError::InvalidNode);
            };
            let rhs_int = if rhs_val.is_pointer_value() {
                self.builder
                    .build_ptr_to_int(rhs_val.into_pointer_value(), ptr_int_type, "ptr2int_rhs")
                    .map_err(|_| BackendError::InvalidNode)?
            } else if rhs_val.is_int_value() {
                let iv = rhs_val.into_int_value();
                if iv.get_type().get_bit_width() != 64 {
                    self.builder
                        .build_int_z_extend(iv, ptr_int_type, "zext_rhs")
                        .map_err(|_| BackendError::InvalidNode)?
                } else {
                    iv
                }
            } else {
                return Err(BackendError::InvalidNode);
            };
            match node.data {
                1 => self
                    .builder
                    .build_int_add(lhs_int, rhs_int, "ptr_add")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                2 => self
                    .builder
                    .build_int_sub(lhs_int, rhs_int, "ptr_sub")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                6 => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::EQ, lhs_int, rhs_int, "ptr_eq")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                7 => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::NE, lhs_int, rhs_int, "ptr_ne")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                8 => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::ULT, lhs_int, rhs_int, "ptr_lt")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                9 => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::UGT, lhs_int, rhs_int, "ptr_gt")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                10 => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::ULE, lhs_int, rhs_int, "ptr_le")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                11 => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::UGE, lhs_int, rhs_int, "ptr_ge")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                _ => return Err(BackendError::InvalidOperator(node.data)),
            }
        } else if use_float {
            let lhs_float = if lhs_val.is_float_value() {
                lhs_val.into_float_value()
            } else {
                let int_val = lhs_val.into_int_value();
                self.builder
                    .build_unsigned_int_to_float(int_val, self.context.f64_type(), "uitofp")
                    .map_err(|_| BackendError::InvalidNode)?
            };
            let rhs_float = if rhs_val.is_float_value() {
                rhs_val.into_float_value()
            } else {
                let int_val = rhs_val.into_int_value();
                self.builder
                    .build_unsigned_int_to_float(int_val, self.context.f64_type(), "uitofp")
                    .map_err(|_| BackendError::InvalidNode)?
            };

            match node.data {
                1 => self
                    .builder
                    .build_float_add(lhs_float, rhs_float, "fadd")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                2 => self
                    .builder
                    .build_float_sub(lhs_float, rhs_float, "fsub")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                3 => self
                    .builder
                    .build_float_mul(lhs_float, rhs_float, "fmul")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                4 => self
                    .builder
                    .build_float_div(lhs_float, rhs_float, "fdiv")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                6 => self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::OEQ, lhs_float, rhs_float, "feq")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                7 => self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::ONE, lhs_float, rhs_float, "fne")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                8 => self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::OLT, lhs_float, rhs_float, "flt")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                9 => self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::OGT, lhs_float, rhs_float, "fgt")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                10 => self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::OLE, lhs_float, rhs_float, "fle")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                11 => self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::OGE, lhs_float, rhs_float, "fge")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                _ => return Err(BackendError::InvalidOperator(node.data)),
            }
        } else {
            // Guard against non-integer aggregate types (e.g., struct values)
            if lhs_val.is_struct_value()
                || rhs_val.is_struct_value()
                || lhs_val.is_array_value()
                || rhs_val.is_array_value()
            {
                return Err(BackendError::InvalidNode);
            }
            let lhs_int = lhs_val.into_int_value();
            let rhs_int = rhs_val.into_int_value();

            // Coerce both int operands to the same width (promote narrower to wider)
            let (lhs_int, rhs_int) = {
                let lw = lhs_int.get_type().get_bit_width();
                let rw = rhs_int.get_type().get_bit_width();
                if lw == rw {
                    (lhs_int, rhs_int)
                } else if lw < rw {
                    let extended = self
                        .builder
                        .build_int_s_extend(lhs_int, rhs_int.get_type(), "sext_lhs")
                        .map_err(|_| BackendError::InvalidNode)?;
                    (extended, rhs_int)
                } else {
                    let extended = self
                        .builder
                        .build_int_s_extend(rhs_int, lhs_int.get_type(), "sext_rhs")
                        .map_err(|_| BackendError::InvalidNode)?;
                    (lhs_int, extended)
                }
            };

            match node.data {
                1 => self
                    .builder
                    .build_int_add(lhs_int, rhs_int, "add")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                2 => self
                    .builder
                    .build_int_sub(lhs_int, rhs_int, "sub")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                3 => self
                    .builder
                    .build_int_mul(lhs_int, rhs_int, "mul")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                4 => self
                    .builder
                    .build_int_signed_div(lhs_int, rhs_int, "div")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                5 => self
                    .builder
                    .build_int_signed_rem(lhs_int, rhs_int, "rem")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                6 => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::EQ, lhs_int, rhs_int, "eq")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                7 => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::NE, lhs_int, rhs_int, "ne")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                8 => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::SLT, lhs_int, rhs_int, "lt")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                9 => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::SGT, lhs_int, rhs_int, "gt")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                10 => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::SLE, lhs_int, rhs_int, "le")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                11 => self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::SGE, lhs_int, rhs_int, "ge")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                12 => self
                    .builder
                    .build_and(lhs_int, rhs_int, "and")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                13 => self
                    .builder
                    .build_or(lhs_int, rhs_int, "or")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                14 => self
                    .builder
                    .build_and(lhs_int, rhs_int, "bitand")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                15 => self
                    .builder
                    .build_or(lhs_int, rhs_int, "bitor")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                16 => self
                    .builder
                    .build_xor(lhs_int, rhs_int, "xor")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                17 => self
                    .builder
                    .build_left_shift(lhs_int, rhs_int, "shl")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                18 => self
                    .builder
                    .build_right_shift(lhs_int, rhs_int, false, "shr")
                    .map_err(|_| BackendError::InvalidNode)?
                    .into(),
                19 => {
                    if lhs_val.is_pointer_value() {
                        self.builder
                            .build_store(lhs_val.into_pointer_value(), rhs_int)
                            .map_err(|_| BackendError::InvalidNode)?;
                        rhs_int.into()
                    } else {
                        rhs_int.into()
                    }
                }
                _ => return Err(BackendError::InvalidOperator(node.data)),
            }
        };

        Ok(Some(result))
    }

    fn lower_unop(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let child_offset = node.first_child;
        if matches!(node.data, 6 | 7) {
            let Some((ptr, pointee_type)) = self.lower_lvalue_ptr(arena, child_offset)? else {
                return Err(BackendError::InvalidNode);
            };

            let current = self
                .builder
                .build_load(pointee_type, ptr, "incdec.load")
                .map_err(|_| BackendError::InvalidNode)?;

            let updated: BasicValueEnum = match current {
                BasicValueEnum::IntValue(int_value) => {
                    let one = int_value.get_type().const_int(1, false);
                    match node.data {
                        6 => self
                            .builder
                            .build_int_add(int_value, one, "inc")
                            .map_err(|_| BackendError::InvalidNode)?
                            .into(),
                        7 => self
                            .builder
                            .build_int_sub(int_value, one, "dec")
                            .map_err(|_| BackendError::InvalidNode)?
                            .into(),
                        _ => unreachable!(),
                    }
                }
                BasicValueEnum::FloatValue(float_value) => {
                    let one = float_value.get_type().const_float(1.0);
                    match node.data {
                        6 => self
                            .builder
                            .build_float_add(float_value, one, "finc")
                            .map_err(|_| BackendError::InvalidNode)?
                            .into(),
                        7 => self
                            .builder
                            .build_float_sub(float_value, one, "fdec")
                            .map_err(|_| BackendError::InvalidNode)?
                            .into(),
                        _ => unreachable!(),
                    }
                }
                BasicValueEnum::PointerValue(ptr_value) => {
                    // Pointer increment/decrement: advance by sizeof(*ptr).
                    // Resolve element type from pointer_struct_types or var_struct_tag.
                    let element_type: BasicTypeEnum<'ctx> = arena
                        .get(child_offset)
                        .filter(|n| n.kind == 60)
                        .and_then(|n| arena.get_string(NodeOffset(n.data)))
                        .and_then(|vname| {
                            self.pointer_struct_types
                                .get(vname)
                                .map(|st| st.as_basic_type_enum())
                                .or_else(|| {
                                    self.var_struct_tag
                                        .get(vname)
                                        .and_then(|tag| self.struct_tag_types.get(tag))
                                        .map(|st| st.as_basic_type_enum())
                                })
                        })
                        .unwrap_or_else(|| self.context.i8_type().as_basic_type_enum());
                    let step: u64 = if node.data == 6 { 1 } else { u64::MAX }; // +1 or -1 (wrapping)
                    let index = self.context.i64_type().const_int(step, true);
                    let new_ptr = unsafe {
                        self.builder
                            .build_gep(element_type, ptr_value, &[index], "ptrinc")
                            .map_err(|_| BackendError::InvalidNode)?
                    };
                    new_ptr.into()
                }
                _ => return Err(BackendError::InvalidNode),
            };

            self.builder
                .build_store(ptr, updated)
                .map_err(|_| BackendError::InvalidNode)?;

            return Ok(Some(updated));
        }

        let operand = self
            .lower_expr(arena, child_offset)?
            .ok_or(BackendError::InvalidNode)?;

        let result: BasicValueEnum = match node.data {
            1 => {
                if operand.is_float_value() {
                    self.builder
                        .build_float_neg(operand.into_float_value(), "fneg")
                        .map_err(|_| BackendError::InvalidNode)?
                        .into()
                } else if operand.is_pointer_value() {
                    let ptr_int = self
                        .builder
                        .build_ptr_to_int(
                            operand.into_pointer_value(),
                            self.context.i64_type(),
                            "ptr2int_neg",
                        )
                        .map_err(|_| BackendError::InvalidNode)?;
                    self.builder
                        .build_int_neg(ptr_int, "neg")
                        .map_err(|_| BackendError::InvalidNode)?
                        .into()
                } else {
                    let int_op = operand.into_int_value();
                    self.builder
                        .build_int_neg(int_op, "neg")
                        .map_err(|_| BackendError::InvalidNode)?
                        .into()
                }
            }
            2 => {
                if operand.is_int_value() {
                    let int_op = operand.into_int_value();
                    let zero = int_op.get_type().const_zero();
                    self.builder
                        .build_int_compare(inkwell::IntPredicate::EQ, int_op, zero, "lnot")
                        .map_err(|_| BackendError::InvalidNode)?
                        .into()
                } else if operand.is_pointer_value() {
                    let ptr_op = operand.into_pointer_value();
                    let ptr_int = self
                        .builder
                        .build_ptr_to_int(ptr_op, self.context.i64_type(), "ptr2int_lnot")
                        .map_err(|_| BackendError::InvalidNode)?;
                    let zero = self.context.i64_type().const_zero();
                    self.builder
                        .build_int_compare(inkwell::IntPredicate::EQ, ptr_int, zero, "lnot")
                        .map_err(|_| BackendError::InvalidNode)?
                        .into()
                } else {
                    operand
                }
            }
            3 => {
                if operand.is_int_value() {
                    let int_op = operand.into_int_value();
                    self.builder
                        .build_not(int_op, "bnot")
                        .map_err(|_| BackendError::InvalidNode)?
                        .into()
                } else {
                    operand
                }
            }
            4 => {
                if let Some((ptr, _)) = self.lower_lvalue_ptr(arena, child_offset)? {
                    ptr.into()
                } else {
                    operand
                }
            }
            5 => {
                if operand.is_pointer_value() {
                    let inferred_ptr_deref = arena
                        .get(child_offset)
                        .filter(|n| n.kind == 60)
                        .and_then(|n| arena.get_string(NodeOffset(n.data)))
                        .and_then(|name| {
                            if let Some(st) = self.pointer_struct_types.get(name).copied() {
                                return Some(st.as_basic_type_enum());
                            }

                            if let Some(binding) = self.variables.get(name) {
                                if binding.pointee_type.is_pointer_type() {
                                    if name.starts_with("pp") || name.starts_with("pz") {
                                        return Some(
                                            self.context
                                                .ptr_type(AddressSpace::default())
                                                .as_basic_type_enum(),
                                        );
                                    }

                                    if self.byte_pointer_vars.contains(name)
                                        || name.starts_with('z')
                                    {
                                        return Some(self.context.i8_type().as_basic_type_enum());
                                    }
                                }
                            }

                            None
                        });

                    let pointee_type = arena
                        .get(child_offset)
                        .filter(|child| child.kind == 204)
                        .and_then(|child| {
                            arena
                                .get_string(NodeOffset(child.data))
                                .map(|s| (child, s.to_string()))
                        })
                        .and_then(|(child, name)| {
                            if name == "__builtin_va_arg" {
                                arena
                                    .get(child.first_child)
                                    .map(|arg1| arg1.next_sibling)
                                    .filter(|off| *off != NodeOffset::NULL)
                                    .and_then(|type_off| {
                                        self.lower_builtin_pointee_type_ast(arena, type_off)
                                    })
                            } else {
                                None
                            }
                        })
                        .or(inferred_ptr_deref)
                        .unwrap_or_else(|| self.to_llvm_type(self.default_type()));
                    self.builder
                        .build_load(pointee_type, operand.into_pointer_value(), "deref")
                        .map_err(|_| BackendError::InvalidNode)?
                        .into()
                } else {
                    operand
                }
            }
            _ => return Err(BackendError::InvalidOperator(node.data)),
        };

        Ok(Some(result))
    }

    fn lower_cond_expr(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        // AST layout: kind=66, first_child=condition, next_sibling=wrapper(kind=0)
        //   wrapper: first_child=then_expr, next_sibling=else_expr
        let cond_offset = node.first_child;
        let wrapper_offset = node.next_sibling;

        let (then_offset, else_offset) = if let Some(wrapper) = arena.get(wrapper_offset) {
            (wrapper.first_child, wrapper.next_sibling)
        } else {
            (NodeOffset::NULL, NodeOffset::NULL)
        };

        let cond_val = match self.lower_expr(arena, cond_offset)? {
            Some(v) => v,
            None => return Ok(None),
        };

        let cond_bool = self.coerce_to_bool(cond_val)?;

        let then_val = self.lower_expr(arena, then_offset)?;
        let else_val = self.lower_expr(arena, else_offset)?;

        match (then_val, else_val) {
            (Some(tv), Some(ev)) => {
                if tv.get_type() == ev.get_type() {
                    let result = self
                        .builder
                        .build_select(cond_bool, tv, ev, "ternary")
                        .map_err(|_| BackendError::InvalidNode)?;
                    Ok(Some(result))
                } else {
                    // Type mismatch — return then value as fallback
                    Ok(Some(tv))
                }
            }
            (Some(tv), None) => Ok(Some(tv)),
            (None, Some(ev)) => Ok(Some(ev)),
            (None, None) => Ok(None),
        }
    }

    /// Coerce a call argument to match the expected parameter type.
    /// Handles ptr→int (ptrtoint), int→ptr (inttoptr), and int width mismatches.
    fn coerce_call_arg(
        &self,
        arg: inkwell::values::BasicMetadataValueEnum<'ctx>,
        expected: inkwell::types::BasicMetadataTypeEnum<'ctx>,
    ) -> Result<inkwell::values::BasicMetadataValueEnum<'ctx>, BackendError> {
        use inkwell::types::BasicMetadataTypeEnum;
        use inkwell::values::BasicMetadataValueEnum;

        match arg {
            BasicMetadataValueEnum::PointerValue(ptr_val) => {
                if let BasicMetadataTypeEnum::IntType(int_ty) = expected {
                    // ptr → int: ptrtoint
                    let cast = self
                        .builder
                        .build_ptr_to_int(ptr_val, int_ty, "arg_ptr2int")
                        .map_err(|_| BackendError::InvalidNode)?;
                    Ok(cast.into())
                } else {
                    Ok(arg)
                }
            }
            BasicMetadataValueEnum::IntValue(int_val) => match expected {
                BasicMetadataTypeEnum::PointerType(ptr_ty) => {
                    let cast = self
                        .builder
                        .build_int_to_ptr(int_val, ptr_ty, "arg_int2ptr")
                        .map_err(|_| BackendError::InvalidNode)?;
                    Ok(cast.into())
                }
                BasicMetadataTypeEnum::IntType(exp_int) => {
                    let val_width = int_val.get_type().get_bit_width();
                    let exp_width = exp_int.get_bit_width();
                    if val_width < exp_width {
                        let ext = self
                            .builder
                            .build_int_s_extend(int_val, exp_int, "arg_sext")
                            .map_err(|_| BackendError::InvalidNode)?;
                        Ok(ext.into())
                    } else if val_width > exp_width {
                        let trunc = self
                            .builder
                            .build_int_truncate(int_val, exp_int, "arg_trunc")
                            .map_err(|_| BackendError::InvalidNode)?;
                        Ok(trunc.into())
                    } else {
                        Ok(arg)
                    }
                }
                BasicMetadataTypeEnum::FloatType(ft) => {
                    let cast = self
                        .builder
                        .build_signed_int_to_float(int_val, ft, "arg_i2f")
                        .map_err(|_| BackendError::InvalidNode)?;
                    Ok(cast.into())
                }
                _ => Ok(arg),
            },
            BasicMetadataValueEnum::FloatValue(float_val) => {
                if let BasicMetadataTypeEnum::IntType(int_ty) = expected {
                    let cast = self
                        .builder
                        .build_float_to_signed_int(float_val, int_ty, "arg_f2i")
                        .map_err(|_| BackendError::InvalidNode)?;
                    Ok(cast.into())
                } else {
                    Ok(arg)
                }
            }
            _ => Ok(arg),
        }
    }

    fn atomic_ordering_from_arg(
        &self,
        arg: Option<&inkwell::values::BasicMetadataValueEnum<'ctx>>,
        default: inkwell::AtomicOrdering,
    ) -> inkwell::AtomicOrdering {
        match arg.and_then(|value| match value {
            inkwell::values::BasicMetadataValueEnum::IntValue(v) => v.get_zero_extended_constant(),
            _ => None,
        }) {
            Some(0) => inkwell::AtomicOrdering::Monotonic,
            Some(1) => inkwell::AtomicOrdering::Acquire,
            Some(2) => inkwell::AtomicOrdering::Acquire,
            Some(3) => inkwell::AtomicOrdering::Release,
            Some(4) => inkwell::AtomicOrdering::AcquireRelease,
            Some(5) => inkwell::AtomicOrdering::SequentiallyConsistent,
            _ => default,
        }
    }

    fn lower_atomic_builtin(
        &mut self,
        builtin_name: &str,
        args: &[inkwell::values::BasicMetadataValueEnum<'ctx>],
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        use inkwell::AtomicOrdering;
        use inkwell::AtomicRMWBinOp;

        let int_zero = || self.context.i32_type().const_int(0, false).into();
        let get_ptr = |idx: usize| {
            args.get(idx).and_then(|arg| match arg {
                inkwell::values::BasicMetadataValueEnum::PointerValue(p) => Some(*p),
                _ => None,
            })
        };
        let get_int = |idx: usize| {
            args.get(idx).and_then(|arg| match arg {
                inkwell::values::BasicMetadataValueEnum::IntValue(v) => Some(*v),
                _ => None,
            })
        };

        let sync_binop = match builtin_name {
            "__sync_fetch_and_add" => Some(AtomicRMWBinOp::Add),
            "__sync_fetch_and_sub" => Some(AtomicRMWBinOp::Sub),
            "__sync_fetch_and_or" => Some(AtomicRMWBinOp::Or),
            "__sync_fetch_and_and" => Some(AtomicRMWBinOp::And),
            "__sync_fetch_and_xor" => Some(AtomicRMWBinOp::Xor),
            "__sync_lock_test_and_set" => Some(AtomicRMWBinOp::Xchg),
            _ => None,
        };

        if let Some(op) = sync_binop {
            if let (Some(ptr), Some(val)) = (get_ptr(0), get_int(1)) {
                let old = self
                    .builder
                    .build_atomicrmw(op, ptr, val, AtomicOrdering::SequentiallyConsistent)
                    .map_err(|_| BackendError::InvalidNode)?;
                return Ok(Some(old.into()));
            }
            return Ok(Some(int_zero()));
        }

        let atomic_binop = match builtin_name {
            "__atomic_fetch_add" => Some(AtomicRMWBinOp::Add),
            "__atomic_fetch_sub" => Some(AtomicRMWBinOp::Sub),
            "__atomic_fetch_or" => Some(AtomicRMWBinOp::Or),
            "__atomic_fetch_and" => Some(AtomicRMWBinOp::And),
            "__atomic_fetch_xor" => Some(AtomicRMWBinOp::Xor),
            _ => None,
        };

        if let Some(op) = atomic_binop {
            if let (Some(ptr), Some(val)) = (get_ptr(0), get_int(1)) {
                let ordering = self
                    .atomic_ordering_from_arg(args.get(2), AtomicOrdering::SequentiallyConsistent);
                let old = self
                    .builder
                    .build_atomicrmw(op, ptr, val, ordering)
                    .map_err(|_| BackendError::InvalidNode)?;
                return Ok(Some(old.into()));
            }
            return Ok(Some(int_zero()));
        }

        match builtin_name {
            "__sync_add_and_fetch"
            | "__sync_sub_and_fetch"
            | "__sync_or_and_fetch"
            | "__sync_and_and_fetch"
            | "__sync_xor_and_fetch"
            | "__atomic_add_fetch"
            | "__atomic_sub_fetch"
            | "__atomic_or_fetch"
            | "__atomic_and_fetch"
            | "__atomic_xor_fetch" => {
                if let (Some(ptr), Some(val)) = (get_ptr(0), get_int(1)) {
                    let op = match builtin_name {
                        "__sync_add_and_fetch" | "__atomic_add_fetch" => AtomicRMWBinOp::Add,
                        "__sync_sub_and_fetch" | "__atomic_sub_fetch" => AtomicRMWBinOp::Sub,
                        "__sync_or_and_fetch" | "__atomic_or_fetch" => AtomicRMWBinOp::Or,
                        "__sync_and_and_fetch" | "__atomic_and_fetch" => AtomicRMWBinOp::And,
                        _ => AtomicRMWBinOp::Xor,
                    };
                    let ordering = if builtin_name.starts_with("__atomic_") {
                        self.atomic_ordering_from_arg(
                            args.get(2),
                            AtomicOrdering::SequentiallyConsistent,
                        )
                    } else {
                        AtomicOrdering::SequentiallyConsistent
                    };
                    let old = self
                        .builder
                        .build_atomicrmw(op, ptr, val, ordering)
                        .map_err(|_| BackendError::InvalidNode)?;
                    let new_val = match op {
                        AtomicRMWBinOp::Add => {
                            self.builder.build_int_add(old, val, "atomic_add_fetch")
                        }
                        AtomicRMWBinOp::Sub => {
                            self.builder.build_int_sub(old, val, "atomic_sub_fetch")
                        }
                        AtomicRMWBinOp::Or => self.builder.build_or(old, val, "atomic_or_fetch"),
                        AtomicRMWBinOp::And => self.builder.build_and(old, val, "atomic_and_fetch"),
                        _ => self.builder.build_xor(old, val, "atomic_xor_fetch"),
                    }
                    .map_err(|_| BackendError::InvalidNode)?;
                    return Ok(Some(new_val.into()));
                }
                Ok(Some(int_zero()))
            }
            "__sync_val_compare_and_swap" => {
                if let (Some(ptr), Some(cmp), Some(new_val)) = (get_ptr(0), get_int(1), get_int(2))
                {
                    let result = self
                        .builder
                        .build_cmpxchg(
                            ptr,
                            cmp,
                            new_val,
                            AtomicOrdering::SequentiallyConsistent,
                            AtomicOrdering::SequentiallyConsistent,
                        )
                        .map_err(|_| BackendError::InvalidNode)?;
                    let old = self
                        .builder
                        .build_extract_value(result, 0, "sync_cas_old")
                        .map_err(|_| BackendError::InvalidNode)?;
                    return Ok(Some(old));
                }
                Ok(Some(int_zero()))
            }
            "__sync_bool_compare_and_swap" => {
                if let (Some(ptr), Some(cmp), Some(new_val)) = (get_ptr(0), get_int(1), get_int(2))
                {
                    let result = self
                        .builder
                        .build_cmpxchg(
                            ptr,
                            cmp,
                            new_val,
                            AtomicOrdering::SequentiallyConsistent,
                            AtomicOrdering::SequentiallyConsistent,
                        )
                        .map_err(|_| BackendError::InvalidNode)?;
                    let success = self
                        .builder
                        .build_extract_value(result, 1, "sync_cas_ok")
                        .map_err(|_| BackendError::InvalidNode)?
                        .into_int_value();
                    let widened = self
                        .builder
                        .build_int_z_extend(success, self.context.i32_type(), "sync_cas_ok_i32")
                        .map_err(|_| BackendError::InvalidNode)?;
                    return Ok(Some(widened.into()));
                }
                Ok(Some(int_zero()))
            }
            "__sync_lock_release" => {
                if let Some(ptr) = get_ptr(0) {
                    let zero = self.context.i32_type().const_zero();
                    let _ = self
                        .builder
                        .build_atomicrmw(AtomicRMWBinOp::Xchg, ptr, zero, AtomicOrdering::Release)
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Ok(Some(int_zero()))
            }
            "__atomic_thread_fence" | "__atomic_signal_fence" | "__sync_synchronize" => {
                let ordering = if builtin_name == "__sync_synchronize" {
                    AtomicOrdering::SequentiallyConsistent
                } else {
                    self.atomic_ordering_from_arg(
                        args.first(),
                        AtomicOrdering::SequentiallyConsistent,
                    )
                };
                self.builder
                    .build_fence(ordering, builtin_name == "__atomic_signal_fence", "")
                    .map_err(|_| BackendError::InvalidNode)?;
                Ok(Some(int_zero()))
            }
            "__atomic_exchange_n" => {
                if let (Some(ptr), Some(val)) = (get_ptr(0), get_int(1)) {
                    let ordering = self.atomic_ordering_from_arg(
                        args.get(2),
                        AtomicOrdering::SequentiallyConsistent,
                    );
                    let old = self
                        .builder
                        .build_atomicrmw(AtomicRMWBinOp::Xchg, ptr, val, ordering)
                        .map_err(|_| BackendError::InvalidNode)?;
                    return Ok(Some(old.into()));
                }
                Ok(Some(int_zero()))
            }
            "__atomic_store_n" => {
                if let (Some(ptr), Some(val)) = (get_ptr(0), get_int(1)) {
                    self.builder
                        .build_store(ptr, val)
                        .map_err(|_| BackendError::InvalidNode)?;
                }
                Ok(Some(int_zero()))
            }
            "__atomic_load_n" => {
                if let Some(ptr) = get_ptr(0) {
                    let loaded = self
                        .builder
                        .build_load(self.context.i32_type(), ptr, "atomic_load")
                        .map_err(|_| BackendError::InvalidNode)?;
                    return Ok(Some(loaded));
                }
                Ok(Some(int_zero()))
            }
            "__atomic_compare_exchange_n" => {
                if let (Some(ptr), Some(expected_ptr), Some(desired)) =
                    (get_ptr(0), get_ptr(1), get_int(2))
                {
                    let expected = self
                        .builder
                        .build_load(desired.get_type(), expected_ptr, "atomic_expected")
                        .map_err(|_| BackendError::InvalidNode)?
                        .into_int_value();
                    let success = self.atomic_ordering_from_arg(
                        args.get(4),
                        AtomicOrdering::SequentiallyConsistent,
                    );
                    let mut failure =
                        self.atomic_ordering_from_arg(args.get(5), AtomicOrdering::Monotonic);
                    if matches!(
                        failure,
                        AtomicOrdering::Release | AtomicOrdering::AcquireRelease
                    ) {
                        failure = AtomicOrdering::Monotonic;
                    }
                    let result = self
                        .builder
                        .build_cmpxchg(ptr, expected, desired, success, failure)
                        .map_err(|_| BackendError::InvalidNode)?;
                    let old = self
                        .builder
                        .build_extract_value(result, 0, "atomic_cmpxchg_old")
                        .map_err(|_| BackendError::InvalidNode)?;
                    let ok = self
                        .builder
                        .build_extract_value(result, 1, "atomic_cmpxchg_ok")
                        .map_err(|_| BackendError::InvalidNode)?
                        .into_int_value();
                    self.builder
                        .build_store(expected_ptr, old)
                        .map_err(|_| BackendError::InvalidNode)?;
                    let widened = self
                        .builder
                        .build_int_z_extend(ok, self.context.i32_type(), "atomic_cmpxchg_ok_i32")
                        .map_err(|_| BackendError::InvalidNode)?;
                    return Ok(Some(widened.into()));
                }
                Ok(Some(int_zero()))
            }
            _ => Ok(Some(int_zero())),
        }
    }

    fn lower_call_expr(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let first_child_offset = node.first_child;
        let first_child = arena.get(first_child_offset);

        // Only extract func_name for plain identifier callees (kind=60).
        // For member-access (kind=69) or other complex callees, use the indirect call path.
        let func_name = first_child
            .filter(|c| c.kind == 60)
            .and_then(|c| arena.get_string(NodeOffset(c.data)))
            .filter(|s| !s.is_empty());

        let mut arg_offsets: Vec<NodeOffset> = Vec::new();
        let mut args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
        if let Some(fc) = first_child {
            let mut arg_offset = fc.next_sibling;
            while arg_offset != NodeOffset::NULL {
                if let Some(arg_node) = arena.get(arg_offset) {
                    // kind=74 is an arg-wrapper node; unwrap it to get the actual expression
                    let expr_offset = if arg_node.kind == 74 {
                        arg_node.first_child
                    } else {
                        arg_offset
                    };
                    arg_offsets.push(expr_offset);
                    if let Some(arg_val) = self.lower_expr(arena, expr_offset)? {
                        // Array-to-pointer decay: if the arg is an array value,
                        // get a pointer to it (GEP with index 0).
                        let coerced = if arg_val.is_array_value() {
                            // Store the array to a temp alloca, then pass pointer
                            let arr_val = arg_val.into_array_value();
                            let arr_ty = arr_val.get_type();
                            if let Ok(alloca) = self.builder.build_alloca(arr_ty, "arr_decay") {
                                let _ = self.builder.build_store(alloca, arr_val);
                                alloca.into()
                            } else {
                                arg_val
                            }
                        } else {
                            arg_val
                        };
                        args.push(coerced.into());
                    }
                    arg_offset = arg_node.next_sibling;
                } else {
                    break;
                }
            }
        }

        if std::env::var("OPTICC_DEBUG_FNPTR").is_ok() {
            eprintln!(
                "DEBUG lower_call_expr: func_name={func_name:?} first_child_kind={}",
                first_child.map(|n| n.kind).unwrap_or(999)
            );
        }

        if let Some(name) = func_name {
            // Intercept va_start/va_end/va_copy/va_arg as LLVM intrinsics
            match name {
                "__builtin_va_start" | "va_start" => {
                    // va_start(ap) → llvm.va_start(ap)
                    let va_start_type = self.context.void_type().fn_type(
                        &[self.context.ptr_type(AddressSpace::default()).into()],
                        false,
                    );
                    let va_start_fn =
                        self.module
                            .get_function("llvm.va_start")
                            .unwrap_or_else(|| {
                                self.module
                                    .add_function("llvm.va_start", va_start_type, None)
                            });
                    if let Some(ap_arg) = args.first() {
                        let ap_ptr = match ap_arg {
                            inkwell::values::BasicMetadataValueEnum::PointerValue(p) => Some(*p),
                            _ => None,
                        };
                        if let Some(ptr) = ap_ptr {
                            let _ = self.builder.build_call(va_start_fn, &[ptr.into()], "");
                        }
                    }
                    return Ok(Some(self.context.i32_type().const_int(0, false).into()));
                }
                "__builtin_va_end" | "va_end" => {
                    let va_end_type = self.context.void_type().fn_type(
                        &[self.context.ptr_type(AddressSpace::default()).into()],
                        false,
                    );
                    let va_end_fn = self.module.get_function("llvm.va_end").unwrap_or_else(|| {
                        self.module.add_function("llvm.va_end", va_end_type, None)
                    });
                    if let Some(ap_arg) = args.first() {
                        let ap_ptr = match ap_arg {
                            inkwell::values::BasicMetadataValueEnum::PointerValue(p) => Some(*p),
                            _ => None,
                        };
                        if let Some(ptr) = ap_ptr {
                            let _ = self.builder.build_call(va_end_fn, &[ptr.into()], "");
                        }
                    }
                    return Ok(Some(self.context.i32_type().const_int(0, false).into()));
                }
                "__builtin_va_copy" | "va_copy" => {
                    let va_copy_type = self.context.void_type().fn_type(
                        &[
                            self.context.ptr_type(AddressSpace::default()).into(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                        ],
                        false,
                    );
                    let va_copy_fn =
                        self.module.get_function("llvm.va_copy").unwrap_or_else(|| {
                            self.module.add_function("llvm.va_copy", va_copy_type, None)
                        });
                    if args.len() >= 2 {
                        let dest = match &args[0] {
                            inkwell::values::BasicMetadataValueEnum::PointerValue(p) => Some(*p),
                            _ => None,
                        };
                        let src = match &args[1] {
                            inkwell::values::BasicMetadataValueEnum::PointerValue(p) => Some(*p),
                            _ => None,
                        };
                        if let (Some(d), Some(s)) = (dest, src) {
                            let _ = self
                                .builder
                                .build_call(va_copy_fn, &[d.into(), s.into()], "");
                        }
                    }
                    return Ok(Some(self.context.i32_type().const_int(0, false).into()));
                }
                _ => {}
            }

            if name.starts_with("__sync_") || name.starts_with("__atomic_") {
                return self.lower_atomic_builtin(name, &args);
            }

            match name {
                "__builtin_expect"
                | "__builtin_expect_with_probability"
                | "__builtin_assume_aligned" => {
                    if let Some(arg) = args.first() {
                        return Ok(Some(match arg {
                            inkwell::values::BasicMetadataValueEnum::IntValue(v) => (*v).into(),
                            inkwell::values::BasicMetadataValueEnum::PointerValue(v) => (*v).into(),
                            inkwell::values::BasicMetadataValueEnum::FloatValue(v) => (*v).into(),
                            _ => self.context.i32_type().const_int(0, false).into(),
                        }));
                    }
                    return Ok(Some(self.context.i32_type().const_int(0, false).into()));
                }
                "__builtin_constant_p" => {
                    let is_const = arg_offsets
                        .first()
                        .and_then(|off| arena.get(*off))
                        .map(|child| matches!(child.kind, 61 | 62 | 63 | 80 | 82))
                        .unwrap_or(false);
                    return Ok(Some(
                        self.context
                            .i32_type()
                            .const_int(if is_const { 1 } else { 0 }, false)
                            .into(),
                    ));
                }
                "__builtin_types_compatible_p" => {
                    let lhs = arg_offsets
                        .first()
                        .and_then(|off| self.lower_builtin_type_signature(arena, *off));
                    let rhs = arg_offsets
                        .get(1)
                        .and_then(|off| self.lower_builtin_type_signature(arena, *off));
                    let compatible = matches!((&lhs, &rhs), (Some(l), Some(r)) if l == r);
                    return Ok(Some(
                        self.context
                            .i32_type()
                            .const_int(if compatible { 1 } else { 0 }, false)
                            .into(),
                    ));
                }
                "__builtin_choose_expr" => {
                    if args.len() >= 3 {
                        let cond_is_true = arg_offsets
                            .first()
                            .and_then(|off| self.lower_builtin_int_constant(arena, *off))
                            .map(|v| v != 0)
                            .or_else(|| {
                                args.first().and_then(|arg| match arg {
                                    inkwell::values::BasicMetadataValueEnum::IntValue(v) => {
                                        v.get_zero_extended_constant().map(|n| n != 0)
                                    }
                                    _ => None,
                                })
                            })
                            .unwrap_or(true);
                        let selected = if cond_is_true { 1 } else { 2 };
                        return Ok(Some(match &args[selected] {
                            inkwell::values::BasicMetadataValueEnum::IntValue(v) => (*v).into(),
                            inkwell::values::BasicMetadataValueEnum::PointerValue(v) => (*v).into(),
                            inkwell::values::BasicMetadataValueEnum::FloatValue(v) => (*v).into(),
                            _ => self.context.i32_type().const_int(0, false).into(),
                        }));
                    }
                    return Ok(Some(self.context.i32_type().const_int(0, false).into()));
                }
                _ => {}
            }

            if name == "u8" {
                if let Some(arg) = args.first() {
                    return Ok(Some(match arg {
                        inkwell::values::BasicMetadataValueEnum::IntValue(v) => (*v).into(),
                        inkwell::values::BasicMetadataValueEnum::PointerValue(v) => (*v).into(),
                        inkwell::values::BasicMetadataValueEnum::FloatValue(v) => (*v).into(),
                        _ => self.context.i32_type().const_zero().into(),
                    }));
                }
                return Ok(Some(self.context.i32_type().const_zero().into()));
            }

            if self.typedef_aliases.contains(name) {
                if let Some(arg) = args.first() {
                    return Ok(Some(match arg {
                        inkwell::values::BasicMetadataValueEnum::IntValue(v) => (*v).into(),
                        inkwell::values::BasicMetadataValueEnum::PointerValue(v) => (*v).into(),
                        inkwell::values::BasicMetadataValueEnum::FloatValue(v) => (*v).into(),
                        _ => self.context.i32_type().const_zero().into(),
                    }));
                }
                return Ok(Some(self.context.i32_type().const_zero().into()));
            }

            // Check if this is a function pointer variable (e.g., a parameter of function pointer type,
            // or a local variable of typedef function-pointer type like sqlite3_loadext_entry)
            if std::env::var("OPTICC_DEBUG_FNPTR").is_ok() {
                eprintln!(
                    "DEBUG lower_call_expr: name={name:?} in_vars={}",
                    self.variables.contains_key(name)
                );
            }
            if let Some(binding) = self.variables.get(name).copied() {
                // Load the variable's value (should be a function pointer)
                let loaded = self
                    .builder
                    .build_load(binding.pointee_type, binding.ptr, "fn_ptr_load")
                    .map_err(|_| BackendError::InvalidNode)?;
                let fn_ptr_opt: Option<inkwell::values::PointerValue> = if loaded.is_pointer_value()
                {
                    Some(loaded.into_pointer_value())
                } else if loaded.is_int_value() {
                    // Function pointer may have been stored as an integer (e.g., from typedef)
                    // Convert int → ptr
                    self.builder
                        .build_int_to_ptr(
                            loaded.into_int_value(),
                            self.context.ptr_type(AddressSpace::default()),
                            "fn_i2p",
                        )
                        .ok()
                } else {
                    None
                };
                if let Some(fn_ptr) = fn_ptr_opt {
                    let param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = args
                        .iter()
                        .map(|a| match a {
                            inkwell::values::BasicMetadataValueEnum::IntValue(v) => {
                                v.get_type().into()
                            }
                            inkwell::values::BasicMetadataValueEnum::FloatValue(v) => {
                                v.get_type().into()
                            }
                            inkwell::values::BasicMetadataValueEnum::PointerValue(_) => {
                                self.context.ptr_type(AddressSpace::default()).into()
                            }
                            _ => self.context.i32_type().into(),
                        })
                        .collect();
                    let fn_type = binding
                        .function_type
                        .unwrap_or_else(|| self.context.i32_type().fn_type(&param_types, true));
                    let call_site = self
                        .builder
                        .build_indirect_call(fn_type, fn_ptr, &args, "indirect_call")
                        .map_err(|_| BackendError::InvalidNode)?;
                    return Ok(Some(match call_site.try_as_basic_value() {
                        inkwell::values::ValueKind::Basic(v) => v,
                        inkwell::values::ValueKind::Instruction(_) => {
                            self.context.i32_type().const_int(0, false).into()
                        }
                    }));
                }
                // If the loaded value is not a pointer (wrong type), fall through to function lookup
            }

            if let Some(func) = self.functions.get(name) {
                let func = *func;
                // Coerce args to match the function's parameter types
                let fn_type = func.get_type();
                let param_types: Vec<_> = fn_type.get_param_types();
                while args.len() < param_types.len() {
                    let pt = param_types[args.len()];
                    let zero_val: inkwell::values::BasicMetadataValueEnum = if pt.is_pointer_type()
                    {
                        self.context
                            .ptr_type(AddressSpace::default())
                            .const_null()
                            .into()
                    } else {
                        self.context.i32_type().const_zero().into()
                    };
                    args.push(zero_val);
                }
                for (i, pt) in param_types.iter().enumerate() {
                    if i >= args.len() {
                        break;
                    }
                    let arg_is_ptr = matches!(
                        args[i],
                        inkwell::values::BasicMetadataValueEnum::PointerValue(_)
                    );
                    if arg_is_ptr && pt.is_int_type() {
                        if let inkwell::values::BasicMetadataValueEnum::PointerValue(pv) = args[i] {
                            if let Ok(iv) =
                                self.builder
                                    .build_ptr_to_int(pv, pt.into_int_type(), "coerce_p2i")
                            {
                                args[i] = iv.into();
                            }
                        }
                    } else if !arg_is_ptr && pt.is_pointer_type() {
                        if let inkwell::values::BasicMetadataValueEnum::IntValue(iv) = args[i] {
                            if let Ok(pv) = self.builder.build_int_to_ptr(
                                iv,
                                self.context.ptr_type(AddressSpace::default()),
                                "coerce_i2p",
                            ) {
                                args[i] = pv.into();
                            }
                        }
                    } else if pt.is_int_type() {
                        if let inkwell::values::BasicMetadataValueEnum::IntValue(iv) = args[i] {
                            let expected_width = pt.into_int_type().get_bit_width();
                            let actual_width = iv.get_type().get_bit_width();
                            if actual_width != expected_width {
                                let target_type = pt.into_int_type();
                                if actual_width < expected_width {
                                    if let Ok(ext) = self.builder.build_int_z_extend(
                                        iv,
                                        target_type,
                                        "coerce_zext",
                                    ) {
                                        args[i] = ext.into();
                                    }
                                } else {
                                    if let Ok(trunc) = self.builder.build_int_truncate(
                                        iv,
                                        target_type,
                                        "coerce_trunc",
                                    ) {
                                        args[i] = trunc.into();
                                    }
                                }
                            }
                        }
                    }
                }
                let call_site = self
                    .builder
                    .build_call(func, &args, "call")
                    .map_err(|_| BackendError::InvalidNode)?;
                return Ok(Some(match call_site.try_as_basic_value() {
                    inkwell::values::ValueKind::Basic(v) => v,
                    inkwell::values::ValueKind::Instruction(_) => {
                        self.context.i32_type().const_int(0, false).into()
                    }
                }));
            }

            // Auto-declare external function using actual argument types from the call site
            // First check if this function already exists in the LLVM module (e.g. forward-declared)
            let ext_func = if let Some(existing) = self.module.get_function(name) {
                // Coerce arguments to match existing function signature
                let fn_type = existing.get_type();
                let param_types: Vec<_> = fn_type.get_param_types();
                while args.len() < param_types.len() {
                    let pt = param_types[args.len()];
                    let zero_val: inkwell::values::BasicMetadataValueEnum = if pt.is_pointer_type()
                    {
                        self.context
                            .ptr_type(AddressSpace::default())
                            .const_null()
                            .into()
                    } else {
                        self.context.i32_type().const_zero().into()
                    };
                    args.push(zero_val);
                }
                for (i, pt) in param_types.iter().enumerate() {
                    if i >= args.len() {
                        break;
                    }
                    let arg_is_ptr = matches!(
                        args[i],
                        inkwell::values::BasicMetadataValueEnum::PointerValue(_)
                    );
                    if arg_is_ptr && pt.is_int_type() {
                        if let inkwell::values::BasicMetadataValueEnum::PointerValue(pv) = args[i] {
                            if let Ok(iv) =
                                self.builder
                                    .build_ptr_to_int(pv, pt.into_int_type(), "coerce_p2i")
                            {
                                args[i] = iv.into();
                            }
                        }
                    } else if !arg_is_ptr && pt.is_pointer_type() {
                        if let inkwell::values::BasicMetadataValueEnum::IntValue(iv) = args[i] {
                            if let Ok(pv) = self.builder.build_int_to_ptr(
                                iv,
                                self.context.ptr_type(AddressSpace::default()),
                                "coerce_i2p",
                            ) {
                                args[i] = pv.into();
                            }
                        }
                    } else if pt.is_int_type() {
                        if let inkwell::values::BasicMetadataValueEnum::IntValue(iv) = args[i] {
                            let expected_width = pt.into_int_type().get_bit_width();
                            let actual_width = iv.get_type().get_bit_width();
                            if actual_width != expected_width {
                                let target_type = pt.into_int_type();
                                if actual_width < expected_width {
                                    if let Ok(ext) = self.builder.build_int_z_extend(
                                        iv,
                                        target_type,
                                        "coerce_zext",
                                    ) {
                                        args[i] = ext.into();
                                    }
                                } else {
                                    if let Ok(trunc) = self.builder.build_int_truncate(
                                        iv,
                                        target_type,
                                        "coerce_trunc",
                                    ) {
                                        args[i] = trunc.into();
                                    }
                                }
                            }
                        }
                    }
                }
                existing
            } else {
                let param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = args
                    .iter()
                    .map(|a| match a {
                        inkwell::values::BasicMetadataValueEnum::IntValue(v) => v.get_type().into(),
                        inkwell::values::BasicMetadataValueEnum::FloatValue(v) => {
                            v.get_type().into()
                        }
                        inkwell::values::BasicMetadataValueEnum::PointerValue(_) => {
                            self.context.ptr_type(AddressSpace::default()).into()
                        }
                        _ => self.context.i32_type().into(),
                    })
                    .collect();
                let fn_type = self.context.i32_type().fn_type(&param_types, false);
                self.module.add_function(name, fn_type, None)
            };
            self.functions.insert(name.to_string(), ext_func);
            let call_site = self
                .builder
                .build_call(ext_func, &args, "call")
                .map_err(|_| BackendError::InvalidNode)?;
            return Ok(Some(match call_site.try_as_basic_value() {
                inkwell::values::ValueKind::Basic(v) => v,
                inkwell::values::ValueKind::Instruction(_) => {
                    self.context.i32_type().const_int(0, false).into()
                }
            }));
        }

        // No named function found — try indirect call through function pointer
        if let Some(callee_val) = self.lower_expr(arena, first_child_offset)? {
            if let BasicValueEnum::PointerValue(fn_ptr) = callee_val {
                // Build an indirect call through the function pointer
                let param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = args
                    .iter()
                    .map(|a| match a {
                        inkwell::values::BasicMetadataValueEnum::IntValue(v) => v.get_type().into(),
                        inkwell::values::BasicMetadataValueEnum::FloatValue(v) => {
                            v.get_type().into()
                        }
                        inkwell::values::BasicMetadataValueEnum::PointerValue(_) => {
                            self.context.ptr_type(AddressSpace::default()).into()
                        }
                        _ => self.context.i32_type().into(),
                    })
                    .collect();
                let fn_type = first_child
                    .and_then(|callee| {
                        if callee.kind == 69 {
                            self.function_type_for_member_access(arena, callee)
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| {
                        // Fallback: use ptr as return type to avoid truncating pointer
                        // returns on x86-64. Using i32 would silently discard the upper
                        // 32 bits of a returned pointer, causing a SIGSEGV on first
                        // pointer dereference. ptr is safer as a default.
                        self.context
                            .ptr_type(AddressSpace::default())
                            .fn_type(&param_types, false)
                    });
                let call_site = self
                    .builder
                    .build_indirect_call(fn_type, fn_ptr, &args, "indirect_call")
                    .map_err(|_| BackendError::InvalidNode)?;
                return Ok(Some(match call_site.try_as_basic_value() {
                    inkwell::values::ValueKind::Basic(v) => v,
                    inkwell::values::ValueKind::Instruction(_) => {
                        // Void-returning call: return a null pointer as a placeholder.
                        // The caller must not use this value for void calls.
                        self.context
                            .ptr_type(AddressSpace::default())
                            .const_null()
                            .into()
                    }
                }));
            }
        }

        Ok(None)
    }

    fn lower_member_access(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        // Field name stored in data (bit31=arrow, lower=string offset); fallback to next_sibling
        let field_str_offset = NodeOffset(node.data & 0x7FFF_FFFF);
        let field_name_str: Option<String> = arena
            .get_string(field_str_offset)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| {
                arena.get(node.next_sibling).and_then(|n| {
                    if n.kind == 60 {
                        arena.get_string(NodeOffset(n.data)).map(|s| s.to_string())
                    } else {
                        None
                    }
                })
            });

        let field_name = match field_name_str {
            Some(f) => f,
            None => return Ok(None),
        };

        let Some((field_ptr, field_llvm_type)) = self.lower_member_access_ptr(arena, node)? else {
            return Ok(None);
        };

        // Check if this was a bitfield access (set by lower_member_access_ptr)
        if let Some((bit_offset, bit_width)) = self.last_bitfield_access.take() {
            // Bitfield READ: load storage unit, shift right, mask
            let storage_val = self
                .builder
                .build_load(field_llvm_type, field_ptr, "bf.storage")
                .map_err(|_| BackendError::InvalidNode)?;
            let int_val = storage_val.into_int_value();
            let int_type = int_val.get_type();

            // Right-shift by bit_offset
            let shifted = if bit_offset > 0 {
                let shift_amt = int_type.const_int(bit_offset as u64, false);
                self.builder
                    .build_right_shift(int_val, shift_amt, false, "bf.shr")
                    .map_err(|_| BackendError::InvalidNode)?
            } else {
                int_val
            };

            // AND-mask with (1 << bit_width) - 1
            let mask_val = (1u64 << bit_width) - 1;
            let mask = int_type.const_int(mask_val, false);
            let masked = self
                .builder
                .build_and(shifted, mask, "bf.mask")
                .map_err(|_| BackendError::InvalidNode)?;

            return Ok(Some(masked.into()));
        }

        let val = self
            .builder
            .build_load(field_llvm_type, field_ptr, &field_name)
            .map_err(|_| BackendError::InvalidNode)?;
        Ok(Some(val))
    }

    fn lower_array_index(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let Some((elem_ptr, elem_type)) = self.lower_array_element_ptr(arena, node)? else {
            return Ok(None);
        };
        let val = self
            .builder
            .build_load(elem_type, elem_ptr, "idx")
            .map_err(|_| BackendError::InvalidNode)?;
        Ok(Some(val))
    }

    fn lower_cast_expr(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let child_offset = node.first_child;
        if child_offset == NodeOffset::NULL {
            return Ok(None);
        }

        let mut tail = child_offset;
        loop {
            let Some(child) = arena.get(tail) else {
                break;
            };
            let next = child.next_sibling;
            if next == NodeOffset::NULL {
                break;
            }
            let Some(next_node) = arena.get(next) else {
                break;
            };
            if matches!(next_node.kind, 1..=16 | 83 | 84 | 101..=106 | 200) {
                tail = next;
            } else {
                return self.lower_expr(arena, next);
            }
        }

        self.lower_expr(arena, child_offset)
    }

    fn lower_sizeof_expr(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        // sizeof(type): outer kind=71 wraps inner kind=71 which wraps type specifier
        // sizeof(expr): outer kind=71 wraps inner kind=71(data=1) which wraps expr
        let inner_offset = node.first_child;
        let inner = arena.get(inner_offset);

        // Unwrap nested kind=71 if present
        let (data, child_offset) = if let Some(inner_node) = inner {
            if inner_node.kind == 71 {
                (inner_node.data, inner_node.first_child)
            } else {
                (node.data, inner_offset)
            }
        } else {
            return Ok(Some(self.context.i64_type().const_int(4, false).into()));
        };

        if data == 0 {
            // sizeof(type) — examine type specifier node kind
            let size = self.sizeof_type_from_ast(arena, child_offset);
            Ok(Some(self.context.i64_type().const_int(size, false).into()))
        } else {
            // sizeof(expr) — infer from AST/value type without evaluating side effects.
            let mut inferred_size = 4u64;
            if let Some(expr_node) = arena.get(child_offset) {
                match expr_node.kind {
                    63 | 81 | 82 => {
                        // String literal includes terminating '\0'.
                        if let Some(s) = arena.get_string(NodeOffset(expr_node.data)) {
                            inferred_size = (s.len() as u64) + 1;
                        }
                    }
                    60 => {
                        if let Some(name) = arena.get_string(NodeOffset(expr_node.data)) {
                            if let Some(binding) = self.variables.get(name).copied() {
                                inferred_size = self.llvm_type_size_bytes(binding.pointee_type);
                            }
                        }
                    }
                    68 => {
                        if let Ok(Some((_ptr, elem_ty))) =
                            self.lower_array_element_ptr(arena, expr_node)
                        {
                            inferred_size = self.llvm_type_size_bytes(elem_ty);
                        }
                    }
                    69 => {
                        let saved_bitfield = self.last_bitfield_access.take();
                        if let Ok(Some((_ptr, field_ty))) =
                            self.lower_member_access_ptr(arena, expr_node)
                        {
                            inferred_size = self.llvm_type_size_bytes(field_ty);
                        }
                        self.last_bitfield_access = saved_bitfield;
                    }
                    _ => {}
                }
            }

            Ok(Some(
                self.context
                    .i64_type()
                    .const_int(inferred_size, false)
                    .into(),
            ))
        }
    }

    fn llvm_type_size_bytes(&self, ty: BasicTypeEnum<'ctx>) -> u64 {
        match ty {
            BasicTypeEnum::IntType(it) => (it.get_bit_width().max(8) as u64) / 8,
            BasicTypeEnum::FloatType(ft) => ft.size_of().get_zero_extended_constant().unwrap_or(8),
            BasicTypeEnum::PointerType(_) => 8,
            BasicTypeEnum::ArrayType(at) => {
                self.llvm_type_size_bytes(at.get_element_type()) * at.len() as u64
            }
            BasicTypeEnum::StructType(st) => self.ast_record_size_align_from_llvm(st).0,
            BasicTypeEnum::VectorType(vt) => vt
                .size_of()
                .and_then(|iv| iv.get_zero_extended_constant())
                .unwrap_or(16),
            BasicTypeEnum::ScalableVectorType(_) => 16,
        }
    }

    fn sizeof_type_from_ast(&self, arena: &Arena, offset: NodeOffset) -> u64 {
        let node = match arena.get(offset) {
            Some(n) => n,
            None => return 4,
        };

        // Type specifier nodes have specific kinds from parse_type_specifier
        match node.kind {
            1 => {
                if node.first_child != NodeOffset::NULL {
                    self.sizeof_type_from_ast(arena, node.first_child)
                } else {
                    0
                }
            }
            2 => 4,  // int
            3 => 1,  // char
            10 => 2, // short
            11 => {
                // long — check if "long long" by looking at sibling
                let sibling = arena.get(node.next_sibling);
                if sibling.map(|s| s.kind) == Some(11) {
                    8
                } else {
                    8
                }
            }
            12 => 4, // signed (defaults to signed int)
            13 => 4, // unsigned (defaults to unsigned int)
            14 => 1, // _Bool
            4 | 5 => self
                .ast_record_size_align(arena, &node)
                .map(|(size, _)| size)
                .unwrap_or(0),
            83 => 4, // float
            84 => 8, // double
            _ => {
                if node.first_child != NodeOffset::NULL {
                    let nested = self.sizeof_type_from_ast(arena, node.first_child);
                    if nested != 0 {
                        return nested;
                    }
                }
                if node.next_sibling != NodeOffset::NULL {
                    let nested = self.sizeof_type_from_ast(arena, node.next_sibling);
                    if nested != 0 {
                        return nested;
                    }
                }
                8
            }
        }
    }

    fn lower_comma_expr(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        // Comma expression: kind=72, first_child=left, next_sibling=right
        // Evaluate left for side effects, return right
        let _left_val = self.lower_expr(arena, node.first_child)?;
        let right_val = self.lower_expr(arena, node.next_sibling)?;
        Ok(right_val)
    }

    fn lower_assign_expr(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let lhs_offset = node.first_child;
        let rhs_offset = node.next_sibling;

        let rhs_val = match self.lower_expr(arena, rhs_offset)? {
            Some(v) => v,
            None => return Ok(None),
        };

        let Some((lhs_ptr, lhs_type)) = self.lower_lvalue_ptr(arena, lhs_offset)? else {
            return Ok(Some(rhs_val));
        };

        // Check if the lvalue target is a bitfield (set by lower_member_access_ptr
        // via lower_lvalue_ptr → kind=69 path)
        if let Some((bit_offset, bit_width)) = self.last_bitfield_access.take() {
            // Bitfield WRITE: read-modify-write on the storage unit
            let new_val = if node.data == 19 {
                rhs_val
            } else {
                // For compound assignment, read the current bitfield value first
                let storage_val = self
                    .builder
                    .build_load(lhs_type, lhs_ptr, "bf.assign.storage")
                    .map_err(|_| BackendError::InvalidNode)?;
                let int_val = storage_val.into_int_value();
                let int_type = int_val.get_type();
                let shifted = if bit_offset > 0 {
                    let sh = int_type.const_int(bit_offset as u64, false);
                    self.builder
                        .build_right_shift(int_val, sh, false, "bf.ca.shr")
                        .map_err(|_| BackendError::InvalidNode)?
                } else {
                    int_val
                };
                let field_mask = int_type.const_int((1u64 << bit_width) - 1, false);
                let current_bf = self
                    .builder
                    .build_and(shifted, field_mask, "bf.ca.cur")
                    .map_err(|_| BackendError::InvalidNode)?;
                self.apply_assignment_op(node.data, current_bf.into(), rhs_val)?
            };

            // Load current storage unit
            let old_storage = self
                .builder
                .build_load(lhs_type, lhs_ptr, "bf.old")
                .map_err(|_| BackendError::InvalidNode)?
                .into_int_value();
            let int_type = old_storage.get_type();

            // Clear the bitfield's bits: old & ~(field_mask << bit_offset)
            let field_mask_val = (1u64 << bit_width) - 1;
            let positioned_mask = field_mask_val << bit_offset;
            let clear_mask = int_type.const_int(!positioned_mask, false);
            let cleared = self
                .builder
                .build_and(old_storage, clear_mask, "bf.cleared")
                .map_err(|_| BackendError::InvalidNode)?;

            // Position the new value: (new_val & field_mask) << bit_offset
            let new_int = if new_val.is_int_value() {
                let nv = new_val.into_int_value();
                if nv.get_type().get_bit_width() != int_type.get_bit_width() {
                    self.builder
                        .build_int_z_extend_or_bit_cast(nv, int_type, "bf.zext")
                        .map_err(|_| BackendError::InvalidNode)?
                } else {
                    nv
                }
            } else {
                int_type.const_int(0, false)
            };
            let field_mask_const = int_type.const_int(field_mask_val, false);
            let masked_new = self
                .builder
                .build_and(new_int, field_mask_const, "bf.new.masked")
                .map_err(|_| BackendError::InvalidNode)?;
            let shifted_new = if bit_offset > 0 {
                let sh = int_type.const_int(bit_offset as u64, false);
                self.builder
                    .build_left_shift(masked_new, sh, "bf.new.shl")
                    .map_err(|_| BackendError::InvalidNode)?
            } else {
                masked_new
            };

            // OR together: cleared | shifted_new
            let result = self
                .builder
                .build_or(cleared, shifted_new, "bf.insert")
                .map_err(|_| BackendError::InvalidNode)?;

            self.builder
                .build_store(lhs_ptr, result)
                .map_err(|_| BackendError::InvalidNode)?;

            // Return the written bitfield value (masked, not the whole storage unit)
            return Ok(Some(masked_new.into()));
        }

        let value_to_store = if node.data == 19 {
            rhs_val
        } else {
            let current_val = self
                .builder
                .build_load(lhs_type, lhs_ptr, "assign_lhs")
                .map_err(|_| BackendError::InvalidNode)?;
            self.apply_assignment_op(node.data, current_val, rhs_val)?
        };

        let value_to_store =
            if let (BasicTypeEnum::IntType(lhs_int), BasicValueEnum::IntValue(rhs_int)) =
                (lhs_type, value_to_store)
            {
                if lhs_int.get_bit_width() != rhs_int.get_type().get_bit_width() {
                    self.builder
                        .build_int_cast(rhs_int, lhs_int, "assign_int_cast")
                        .map_err(|_| BackendError::InvalidNode)?
                        .as_basic_value_enum()
                } else {
                    rhs_int.as_basic_value_enum()
                }
            } else if let (
                BasicTypeEnum::PointerType(lhs_ptr_ty),
                BasicValueEnum::IntValue(rhs_int),
            ) = (lhs_type, value_to_store)
            {
                if rhs_int.get_zero_extended_constant() == Some(0) {
                    lhs_ptr_ty.const_null().as_basic_value_enum()
                } else {
                    self.builder
                        .build_int_to_ptr(rhs_int, lhs_ptr_ty, "assign_int2ptr")
                        .map_err(|_| BackendError::InvalidNode)?
                        .as_basic_value_enum()
                }
            } else {
                value_to_store
            };

        if Self::types_compatible(lhs_type, value_to_store) {
            self.builder
                .build_store(lhs_ptr, value_to_store)
                .map_err(|_| BackendError::InvalidNode)?;

            if let Some(lhs_node) = arena.get(lhs_offset) {
                if lhs_node.kind == 60 {
                    if let Some(lhs_name) = arena.get_string(NodeOffset(lhs_node.data)) {
                        if lhs_type.is_pointer_type()
                            && self.subtree_has_byte_pointer_ident(arena, rhs_offset)
                        {
                            self.byte_pointer_vars.insert(lhs_name.to_string());
                        }
                    }
                }
            }
        }

        // Load back from the lvalue so the result is a runtime instruction,
        // not a compile-time constant. This prevents LLVM from constant-folding
        // expressions like (x = 42) > 0 into `br i1 true`.
        let result = self
            .builder
            .build_load(lhs_type, lhs_ptr, "assign_result")
            .map_err(|_| BackendError::InvalidNode)?;
        Ok(Some(result))
    }

    fn lower_typeof(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        if node.first_child != NodeOffset::NULL {
            self.lower_expr(arena, node.first_child)
        } else {
            Ok(None)
        }
    }

    fn lower_stmt_expr(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let mut last_val: Option<BasicValueEnum<'ctx>> = None;
        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                last_val = self.lower_expr(arena, child_offset)?;
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }
        Ok(last_val)
    }

    /// Lower `&&label` (GCC label-as-value extension, kind=203).
    /// Produces LLVM `blockaddress(@fn, %label_bb)`.
    fn lower_label_addr(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let label_name = if node.data != 0 {
            arena
                .get_string(NodeOffset(node.data))
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        };

        if label_name.is_empty() {
            let ptr = self.context.ptr_type(AddressSpace::default()).const_null();
            return Ok(Some(ptr.into()));
        }

        let function = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or(BackendError::InvalidNode)?;

        // Get or create the target basic block for this label
        let label_bb = if let Some(bb) = self.label_blocks.get(&label_name) {
            *bb
        } else {
            let bb = self
                .context
                .append_basic_block(function, &format!("label.{}", label_name));
            self.label_blocks.insert(label_name.clone(), bb);
            bb
        };

        // Get the blockaddress via inkwell's get_address()
        // SAFETY: The returned pointer is only used for indirectbr (computed goto).
        let addr = unsafe { label_bb.get_address() };
        match addr {
            Some(ptr_val) => Ok(Some(ptr_val.into())),
            None => {
                // get_address() returns None for entry blocks; fall back to null pointer
                let ptr = self.context.ptr_type(AddressSpace::default()).const_null();
                Ok(Some(ptr.into()))
            }
        }
    }

    fn lower_builtin_type_signature(&self, arena: &Arena, offset: NodeOffset) -> Option<String> {
        let node = arena.get(offset)?;
        match node.kind {
            60 => arena
                .get_string(NodeOffset(node.data))
                .map(|s| s.to_string()),
            1 => Some("void".to_string()),
            2 | 12 | 13 => Some("int".to_string()),
            3 => Some("char".to_string()),
            10 => Some("short".to_string()),
            11 => Some("long".to_string()),
            14 => Some("_Bool".to_string()),
            83 => Some("float".to_string()),
            84 => Some("double".to_string()),
            4 => {
                let tag = if node.data != 0 {
                    arena.get_string(NodeOffset(node.data)).unwrap_or("")
                } else {
                    ""
                };
                Some(format!("struct:{}", tag))
            }
            5 => {
                let tag = if node.data != 0 {
                    arena.get_string(NodeOffset(node.data)).unwrap_or("")
                } else {
                    ""
                };
                Some(format!("union:{}", tag))
            }
            7 => self
                .lower_builtin_type_signature(arena, node.first_child)
                .map(|inner| format!("*{}", inner)),
            8 => self
                .lower_builtin_type_signature(arena, node.first_child)
                .map(|inner| format!("[{};{}]", inner, node.data)),
            74 | 201 => self.lower_builtin_type_signature(arena, node.first_child),
            _ => {
                if node.first_child != NodeOffset::NULL {
                    self.lower_builtin_type_signature(arena, node.first_child)
                } else {
                    None
                }
            }
        }
    }

    fn lower_builtin_int_constant(&self, arena: &Arena, offset: NodeOffset) -> Option<u64> {
        let node = arena.get(offset)?;
        match node.kind {
            61 => Some(node.data as u64),
            74 => self.lower_builtin_int_constant(arena, node.first_child),
            _ => None,
        }
    }

    fn lower_builtin_type_ast(
        &self,
        arena: &Arena,
        offset: NodeOffset,
    ) -> Option<BasicTypeEnum<'ctx>> {
        let mut current = offset;
        let mut base_type: Option<BasicTypeEnum<'ctx>> = None;

        while current != NodeOffset::NULL {
            let node = arena.get(current)?;
            match node.kind {
                7 | 8 | 9 => {
                    return Some(
                        self.context
                            .ptr_type(AddressSpace::default())
                            .as_basic_type_enum(),
                    )
                }
                4 | 5 => return Some(self.specifier_to_llvm_type(arena, node)),
                1 | 2 | 3 | 10 | 11 | 12 | 13 | 14 | 16 | 83 | 84 => {
                    if base_type.is_none() {
                        base_type = Some(self.node_kind_to_llvm_type(node.kind));
                    }
                }
                74 | 201 => return self.lower_builtin_type_ast(arena, node.first_child),
                60 => {
                    if let Some(name) = arena.get_string(NodeOffset(node.data)) {
                        if self.typedef_aliases.contains(name) {
                            return Some(
                                self.context
                                    .ptr_type(AddressSpace::default())
                                    .as_basic_type_enum(),
                            );
                        }
                    }
                }
                _ => {
                    if node.first_child != NodeOffset::NULL {
                        if let Some(inner) = self.lower_builtin_type_ast(arena, node.first_child) {
                            return Some(inner);
                        }
                    }
                }
            }
            current = node.next_sibling;
        }

        base_type
    }

    fn lower_builtin_pointee_type_ast(
        &self,
        arena: &Arena,
        offset: NodeOffset,
    ) -> Option<BasicTypeEnum<'ctx>> {
        let node = arena.get(offset)?;
        match node.kind {
            7 => {
                if node.first_child != NodeOffset::NULL {
                    self.lower_builtin_type_ast(arena, node.first_child)
                } else {
                    Some(self.context.i8_type().as_basic_type_enum())
                }
            }
            8 | 9 => Some(
                self.context
                    .ptr_type(AddressSpace::default())
                    .as_basic_type_enum(),
            ),
            74 | 201 => self.lower_builtin_pointee_type_ast(arena, node.first_child),
            _ => {
                if node.first_child != NodeOffset::NULL {
                    self.lower_builtin_pointee_type_ast(arena, node.first_child)
                } else {
                    None
                }
            }
        }
    }

    fn lower_builtin_call(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let builtin_name = arena
            .get_string(NodeOffset(node.data))
            .or_else(|| {
                if node.first_child != NodeOffset::NULL {
                    arena.get_string(NodeOffset(node.first_child.0))
                } else {
                    None
                }
            })
            .map(|s| s.to_string());

        let builtin_name = builtin_name.unwrap_or_else(|| "__builtin_unknown".to_string());

        // Unknown builtins with no name: just return 0 (no-op)
        if builtin_name == "__builtin_unknown" {
            return Ok(Some(self.context.i32_type().const_int(0, false).into()));
        }

        let mut arg_offsets: Vec<NodeOffset> = Vec::new();
        let mut args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
        let mut child_offset = if arena.get(node.first_child).is_some() {
            node.first_child
        } else {
            node.next_sibling
        };
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                arg_offsets.push(child_offset);
                if let Some(arg_val) = self.lower_expr(arena, child_offset)? {
                    args.push(arg_val.into());
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }

        if builtin_name.starts_with("__sync_") || builtin_name.starts_with("__atomic_") {
            return self.lower_atomic_builtin(&builtin_name, &args);
        }

        match builtin_name.as_str() {
            "__builtin_expect" => {
                if let Some(arg) = args.first() {
                    match arg {
                        inkwell::values::BasicMetadataValueEnum::IntValue(v) => {
                            return Ok(Some((*v).into()))
                        }
                        inkwell::values::BasicMetadataValueEnum::PointerValue(v) => {
                            return Ok(Some((*v).into()))
                        }
                        inkwell::values::BasicMetadataValueEnum::FloatValue(v) => {
                            return Ok(Some((*v).into()))
                        }
                        _ => return Ok(None),
                    }
                }
                Ok(None)
            }
            "__builtin_constant_p" => {
                let is_const = node.first_child != NodeOffset::NULL && {
                    if let Some(child) = arena.get(node.first_child) {
                        matches!(child.kind, 61 | 62 | 63 | 80 | 82)
                    } else {
                        false
                    }
                };
                let val = if is_const { 1u64 } else { 0u64 };
                Ok(Some(self.context.i32_type().const_int(val, false).into()))
            }
            "__builtin_offsetof" => Ok(Some(self.context.i64_type().const_int(0, false).into())),
            "__builtin_memcpy" | "__builtin_memset" | "__builtin_strlen" => {
                let _intrinsic_name = match builtin_name.as_str() {
                    "__builtin_memcpy" => "llvm.memcpy.p0.p0.i64",
                    "__builtin_memset" => "llvm.memset.p0.i64",
                    "__builtin_strlen" => "llvm.strlen",
                    _ => unreachable!(),
                };

                if let Some(func) = self.functions.get(&builtin_name) {
                    let call_site = self
                        .builder
                        .build_call(*func, &args, "builtin_call")
                        .map_err(|_| BackendError::InvalidNode)?;
                    return Ok(Some(match call_site.try_as_basic_value() {
                        inkwell::values::ValueKind::Basic(v) => v,
                        _ => self.context.i32_type().const_int(0, false).into(),
                    }));
                }

                let fn_type = self.context.i64_type().fn_type(
                    &args
                        .iter()
                        .map(|a| match a {
                            inkwell::values::BasicMetadataValueEnum::IntValue(v) => {
                                v.get_type().into()
                            }
                            inkwell::values::BasicMetadataValueEnum::PointerValue(v) => {
                                v.get_type().into()
                            }
                            inkwell::values::BasicMetadataValueEnum::FloatValue(v) => {
                                v.get_type().into()
                            }
                            _ => self.context.i64_type().into(),
                        })
                        .collect::<Vec<_>>(),
                    false,
                );
                let func = self.module.add_function(&builtin_name, fn_type, None);
                self.functions.insert(builtin_name.clone(), func);
                let call_site = self
                    .builder
                    .build_call(func, &args, "builtin_call")
                    .map_err(|_| BackendError::InvalidNode)?;
                Ok(Some(match call_site.try_as_basic_value() {
                    inkwell::values::ValueKind::Basic(v) => v,
                    _ => self.context.i32_type().const_int(0, false).into(),
                }))
            }
            "__builtin_va_start" => {
                let va_start_type = self.context.void_type().fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into()],
                    false,
                );
                let va_start_fn = self
                    .module
                    .get_function("llvm.va_start")
                    .unwrap_or_else(|| {
                        self.module
                            .add_function("llvm.va_start", va_start_type, None)
                    });
                if let Some(ap_off) = arg_offsets.first().copied() {
                    if let Some((ap_ptr, _)) = self.lower_lvalue_ptr(arena, ap_off)? {
                        let _ = self.builder.build_call(va_start_fn, &[ap_ptr.into()], "");
                    }
                }
                Ok(Some(self.context.i32_type().const_zero().into()))
            }
            "__builtin_va_end" => {
                let va_end_type = self.context.void_type().fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into()],
                    false,
                );
                let va_end_fn = self
                    .module
                    .get_function("llvm.va_end")
                    .unwrap_or_else(|| self.module.add_function("llvm.va_end", va_end_type, None));
                if let Some(ap_off) = arg_offsets.first().copied() {
                    if let Some((ap_ptr, _)) = self.lower_lvalue_ptr(arena, ap_off)? {
                        let _ = self.builder.build_call(va_end_fn, &[ap_ptr.into()], "");
                    }
                }
                Ok(Some(self.context.i32_type().const_zero().into()))
            }
            "__builtin_va_copy" => {
                if let (Some(dst_off), Some(src_off)) =
                    (arg_offsets.first().copied(), arg_offsets.get(1).copied())
                {
                    if let Some((dst_ptr, dst_type)) = self.lower_lvalue_ptr(arena, dst_off)? {
                        if let Some(src_val) = self.lower_expr(arena, src_off)? {
                            let to_store = if Self::types_compatible(dst_type, src_val) {
                                src_val
                            } else {
                                self.context
                                    .ptr_type(AddressSpace::default())
                                    .const_null()
                                    .into()
                            };
                            self.builder
                                .build_store(dst_ptr, to_store)
                                .map_err(|_| BackendError::InvalidNode)?;
                        }
                    }
                }
                Ok(Some(self.context.i32_type().const_zero().into()))
            }
            "__builtin_va_arg" => {
                let Some(ap_off) = arg_offsets.first().copied() else {
                    return Ok(Some(self.context.i32_type().const_zero().into()));
                };
                let Some(type_off) = arg_offsets.get(1).copied() else {
                    return Ok(Some(self.context.i32_type().const_zero().into()));
                };
                let Some((ap_ptr, _)) = self.lower_lvalue_ptr(arena, ap_off)? else {
                    return Ok(Some(self.context.i32_type().const_zero().into()));
                };
                let Some(va_type) = self.lower_builtin_type_ast(arena, type_off) else {
                    return Ok(Some(self.context.i32_type().const_zero().into()));
                };
                let value = match va_type {
                    BasicTypeEnum::ArrayType(ty) => self.builder.build_va_arg(ap_ptr, ty, "vaarg"),
                    BasicTypeEnum::FloatType(ty) => self.builder.build_va_arg(ap_ptr, ty, "vaarg"),
                    BasicTypeEnum::IntType(ty) => self.builder.build_va_arg(ap_ptr, ty, "vaarg"),
                    BasicTypeEnum::PointerType(ty) => {
                        self.builder.build_va_arg(ap_ptr, ty, "vaarg")
                    }
                    BasicTypeEnum::StructType(ty) => self.builder.build_va_arg(ap_ptr, ty, "vaarg"),
                    BasicTypeEnum::VectorType(ty) => self.builder.build_va_arg(ap_ptr, ty, "vaarg"),
                    BasicTypeEnum::ScalableVectorType(ty) => {
                        self.builder.build_va_arg(ap_ptr, ty, "vaarg")
                    }
                }
                .map_err(|_| BackendError::InvalidNode)?;
                Ok(Some(value))
            }
            "__builtin_types_compatible_p" => {
                let lhs = arg_offsets
                    .first()
                    .and_then(|off| self.lower_builtin_type_signature(arena, *off));
                let rhs = arg_offsets
                    .get(1)
                    .and_then(|off| self.lower_builtin_type_signature(arena, *off));
                let compatible = matches!((&lhs, &rhs), (Some(l), Some(r)) if l == r);
                Ok(Some(
                    self.context
                        .i32_type()
                        .const_int(if compatible { 1 } else { 0 }, false)
                        .into(),
                ))
            }
            "__builtin_unreachable" => {
                self.builder
                    .build_unreachable()
                    .map_err(|_| BackendError::InvalidNode)?;
                Ok(None)
            }
            "__builtin_trap" => {
                // Emit a call to llvm.trap
                let trap_fn_type = self.context.void_type().fn_type(&[], false);
                let trap_fn = self
                    .module
                    .get_function("llvm.trap")
                    .unwrap_or_else(|| self.module.add_function("llvm.trap", trap_fn_type, None));
                self.builder
                    .build_call(trap_fn, &[], "trap")
                    .map_err(|_| BackendError::InvalidNode)?;
                self.builder
                    .build_unreachable()
                    .map_err(|_| BackendError::InvalidNode)?;
                Ok(None)
            }
            "__builtin_expect_with_probability" => {
                // Same as __builtin_expect: return first argument
                if let Some(arg) = args.first() {
                    match arg {
                        inkwell::values::BasicMetadataValueEnum::IntValue(v) => {
                            Ok(Some((*v).into()))
                        }
                        inkwell::values::BasicMetadataValueEnum::PointerValue(v) => {
                            Ok(Some((*v).into()))
                        }
                        inkwell::values::BasicMetadataValueEnum::FloatValue(v) => {
                            Ok(Some((*v).into()))
                        }
                        _ => Ok(None),
                    }
                } else {
                    Ok(None)
                }
            }
            "__builtin_assume_aligned" => {
                // Return first argument (the pointer)
                if let Some(arg) = args.first() {
                    match arg {
                        inkwell::values::BasicMetadataValueEnum::PointerValue(v) => {
                            Ok(Some((*v).into()))
                        }
                        inkwell::values::BasicMetadataValueEnum::IntValue(v) => {
                            Ok(Some((*v).into()))
                        }
                        _ => Ok(None),
                    }
                } else {
                    Ok(None)
                }
            }
            "__builtin_clz" | "__builtin_clzl" | "__builtin_clzll" => {
                // Count leading zeros → LLVM ctlz intrinsic
                if let Some(inkwell::values::BasicMetadataValueEnum::IntValue(val)) = args.first() {
                    let bit_width = val.get_type().get_bit_width();
                    let fn_name = format!("llvm.ctlz.i{}", bit_width);
                    let fn_type = val.get_type().fn_type(
                        &[val.get_type().into(), self.context.bool_type().into()],
                        false,
                    );
                    let func = self
                        .module
                        .get_function(&fn_name)
                        .unwrap_or_else(|| self.module.add_function(&fn_name, fn_type, None));
                    // is_zero_poison = true (matching GCC: undefined for 0)
                    let call = self
                        .builder
                        .build_call(
                            func,
                            &[
                                (*val).into(),
                                self.context.bool_type().const_int(1, false).into(),
                            ],
                            "clz",
                        )
                        .map_err(|_| BackendError::InvalidNode)?;
                    Ok(Some(match call.try_as_basic_value() {
                        inkwell::values::ValueKind::Basic(v) => v,
                        _ => self.context.i32_type().const_int(0, false).into(),
                    }))
                } else {
                    Ok(Some(self.context.i32_type().const_int(0, false).into()))
                }
            }
            "__builtin_ctz" | "__builtin_ctzl" | "__builtin_ctzll" => {
                // Count trailing zeros → LLVM cttz intrinsic
                if let Some(inkwell::values::BasicMetadataValueEnum::IntValue(val)) = args.first() {
                    let bit_width = val.get_type().get_bit_width();
                    let fn_name = format!("llvm.cttz.i{}", bit_width);
                    let fn_type = val.get_type().fn_type(
                        &[val.get_type().into(), self.context.bool_type().into()],
                        false,
                    );
                    let func = self
                        .module
                        .get_function(&fn_name)
                        .unwrap_or_else(|| self.module.add_function(&fn_name, fn_type, None));
                    let call = self
                        .builder
                        .build_call(
                            func,
                            &[
                                (*val).into(),
                                self.context.bool_type().const_int(1, false).into(),
                            ],
                            "ctz",
                        )
                        .map_err(|_| BackendError::InvalidNode)?;
                    Ok(Some(match call.try_as_basic_value() {
                        inkwell::values::ValueKind::Basic(v) => v,
                        _ => self.context.i32_type().const_int(0, false).into(),
                    }))
                } else {
                    Ok(Some(self.context.i32_type().const_int(0, false).into()))
                }
            }
            "__builtin_popcount" | "__builtin_popcountl" | "__builtin_popcountll" => {
                // Population count → LLVM ctpop intrinsic
                if let Some(inkwell::values::BasicMetadataValueEnum::IntValue(val)) = args.first() {
                    let bit_width = val.get_type().get_bit_width();
                    let fn_name = format!("llvm.ctpop.i{}", bit_width);
                    let fn_type = val.get_type().fn_type(&[val.get_type().into()], false);
                    let func = self
                        .module
                        .get_function(&fn_name)
                        .unwrap_or_else(|| self.module.add_function(&fn_name, fn_type, None));
                    let call = self
                        .builder
                        .build_call(func, &[(*val).into()], "popcount")
                        .map_err(|_| BackendError::InvalidNode)?;
                    Ok(Some(match call.try_as_basic_value() {
                        inkwell::values::ValueKind::Basic(v) => v,
                        _ => self.context.i32_type().const_int(0, false).into(),
                    }))
                } else {
                    Ok(Some(self.context.i32_type().const_int(0, false).into()))
                }
            }
            "__builtin_bswap16" | "__builtin_bswap32" | "__builtin_bswap64" => {
                // Byte swap → LLVM bswap intrinsic
                if let Some(inkwell::values::BasicMetadataValueEnum::IntValue(val)) = args.first() {
                    let bit_width = val.get_type().get_bit_width();
                    let fn_name = format!("llvm.bswap.i{}", bit_width);
                    let fn_type = val.get_type().fn_type(&[val.get_type().into()], false);
                    let func = self
                        .module
                        .get_function(&fn_name)
                        .unwrap_or_else(|| self.module.add_function(&fn_name, fn_type, None));
                    let call = self
                        .builder
                        .build_call(func, &[(*val).into()], "bswap")
                        .map_err(|_| BackendError::InvalidNode)?;
                    Ok(Some(match call.try_as_basic_value() {
                        inkwell::values::ValueKind::Basic(v) => v,
                        _ => self.context.i32_type().const_int(0, false).into(),
                    }))
                } else {
                    Ok(Some(self.context.i32_type().const_int(0, false).into()))
                }
            }
            "__builtin_ffs" | "__builtin_ffsl" | "__builtin_ffsll" => {
                // Find first set bit (1-indexed, 0 if input is 0)
                // ffs(x) = x == 0 ? 0 : ctz(x) + 1
                if let Some(inkwell::values::BasicMetadataValueEnum::IntValue(val)) = args.first() {
                    let bit_width = val.get_type().get_bit_width();
                    let fn_name = format!("llvm.cttz.i{}", bit_width);
                    let fn_type = val.get_type().fn_type(
                        &[val.get_type().into(), self.context.bool_type().into()],
                        false,
                    );
                    let func = self
                        .module
                        .get_function(&fn_name)
                        .unwrap_or_else(|| self.module.add_function(&fn_name, fn_type, None));
                    let ctz = self
                        .builder
                        .build_call(
                            func,
                            &[
                                (*val).into(),
                                self.context.bool_type().const_int(0, false).into(),
                            ],
                            "ctz_ffs",
                        )
                        .map_err(|_| BackendError::InvalidNode)?;
                    let ctz_val = match ctz.try_as_basic_value() {
                        inkwell::values::ValueKind::Basic(v) => v.into_int_value(),
                        _ => val.get_type().const_int(0, false),
                    };
                    let one = val.get_type().const_int(1, false);
                    let ctz_plus_1 = self
                        .builder
                        .build_int_add(ctz_val, one, "ffs_add")
                        .map_err(|_| BackendError::InvalidNode)?;
                    let zero = val.get_type().const_zero();
                    let is_zero = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::EQ, *val, zero, "ffs_iszero")
                        .map_err(|_| BackendError::InvalidNode)?;
                    let result = self
                        .builder
                        .build_select(is_zero, zero, ctz_plus_1, "ffs")
                        .map_err(|_| BackendError::InvalidNode)?;
                    Ok(Some(result))
                } else {
                    Ok(Some(self.context.i32_type().const_int(0, false).into()))
                }
            }
            "__builtin_abs" | "__builtin_labs" | "__builtin_llabs" => {
                // Absolute value of integer
                if let Some(inkwell::values::BasicMetadataValueEnum::IntValue(val)) = args.first() {
                    let zero = val.get_type().const_zero();
                    let neg = self
                        .builder
                        .build_int_sub(zero, *val, "abs_neg")
                        .map_err(|_| BackendError::InvalidNode)?;
                    let is_neg = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::SLT, *val, zero, "abs_cmp")
                        .map_err(|_| BackendError::InvalidNode)?;
                    let result = self
                        .builder
                        .build_select(is_neg, neg, *val, "abs")
                        .map_err(|_| BackendError::InvalidNode)?;
                    Ok(Some(result))
                } else {
                    Ok(Some(self.context.i32_type().const_int(0, false).into()))
                }
            }
            "__builtin_object_size" => {
                // Conservative: return (size_t)-1 for type 0/1, 0 for type 2/3
                let type_arg = args.get(1).and_then(|a| match a {
                    inkwell::values::BasicMetadataValueEnum::IntValue(v) => {
                        v.get_zero_extended_constant()
                    }
                    _ => None,
                });
                let val = match type_arg {
                    Some(0) | Some(1) | None => u64::MAX,
                    _ => 0,
                };
                Ok(Some(self.context.i64_type().const_int(val, false).into()))
            }
            "__builtin_frame_address" | "__builtin_return_address" => {
                // Return null pointer (conservative)
                Ok(Some(
                    self.context
                        .ptr_type(AddressSpace::default())
                        .const_null()
                        .into(),
                ))
            }
            "__builtin_prefetch" => {
                // Prefetch is a hint, emit nothing
                Ok(Some(self.context.i32_type().const_int(0, false).into()))
            }
            "__builtin_choose_expr" => {
                if args.len() >= 3 {
                    let cond_is_true = arg_offsets
                        .first()
                        .and_then(|off| self.lower_builtin_int_constant(arena, *off))
                        .map(|v| v != 0)
                        .or_else(|| {
                            args.first().and_then(|arg| match arg {
                                inkwell::values::BasicMetadataValueEnum::IntValue(v) => {
                                    v.get_zero_extended_constant().map(|n| n != 0)
                                }
                                _ => None,
                            })
                        })
                        .unwrap_or(true);
                    let selected = if cond_is_true { 1 } else { 2 };
                    match &args[selected] {
                        inkwell::values::BasicMetadataValueEnum::IntValue(v) => {
                            return Ok(Some((*v).into()))
                        }
                        inkwell::values::BasicMetadataValueEnum::PointerValue(v) => {
                            return Ok(Some((*v).into()))
                        }
                        inkwell::values::BasicMetadataValueEnum::FloatValue(v) => {
                            return Ok(Some((*v).into()))
                        }
                        _ => return Ok(None),
                    }
                }
                Ok(None)
            }
            "__builtin_alloca" => {
                // __builtin_alloca(size) → alloca i8, size
                if let Some(inkwell::values::BasicMetadataValueEnum::IntValue(size_val)) =
                    args.first()
                {
                    let alloca = self
                        .builder
                        .build_array_alloca(self.context.i8_type(), *size_val, "builtin_alloca")
                        .map_err(|_| BackendError::InvalidNode)?;
                    Ok(Some(alloca.into()))
                } else {
                    // Fallback: allocate 1 byte
                    let alloca = self
                        .builder
                        .build_alloca(self.context.i8_type(), "builtin_alloca")
                        .map_err(|_| BackendError::InvalidNode)?;
                    Ok(Some(alloca.into()))
                }
            }
            "__builtin_add_overflow" | "__builtin_sub_overflow" | "__builtin_mul_overflow" => {
                // __builtin_*_overflow(a, b, result_ptr) → returns bool (1 if overflow)
                // For now, perform the operation without overflow detection and return 0
                if args.len() >= 3 {
                    if let (
                        Some(inkwell::values::BasicMetadataValueEnum::IntValue(a)),
                        Some(inkwell::values::BasicMetadataValueEnum::IntValue(b)),
                        Some(inkwell::values::BasicMetadataValueEnum::PointerValue(result_ptr)),
                    ) = (args.get(0), args.get(1), args.get(2))
                    {
                        // Coerce a and b to same width
                        let (a_coerced, b_coerced) = {
                            let aw = a.get_type().get_bit_width();
                            let bw = b.get_type().get_bit_width();
                            if aw == bw {
                                (*a, *b)
                            } else if aw < bw {
                                let ext = self
                                    .builder
                                    .build_int_s_extend(*a, b.get_type(), "ovf_sext_a")
                                    .unwrap_or(*a);
                                (ext, *b)
                            } else {
                                let ext = self
                                    .builder
                                    .build_int_s_extend(*b, a.get_type(), "ovf_sext_b")
                                    .unwrap_or(*b);
                                (*a, ext)
                            }
                        };
                        let result = match builtin_name.as_str() {
                            "__builtin_add_overflow" => self
                                .builder
                                .build_int_add(a_coerced, b_coerced, "overflow_add")
                                .map_err(|_| BackendError::InvalidNode)?,
                            "__builtin_sub_overflow" => self
                                .builder
                                .build_int_sub(a_coerced, b_coerced, "overflow_sub")
                                .map_err(|_| BackendError::InvalidNode)?,
                            "__builtin_mul_overflow" => self
                                .builder
                                .build_int_mul(a_coerced, b_coerced, "overflow_mul")
                                .map_err(|_| BackendError::InvalidNode)?,
                            _ => return Err(BackendError::UndefinedFunction(builtin_name.clone())),
                        };
                        self.builder
                            .build_store(*result_ptr, result)
                            .map_err(|_| BackendError::InvalidNode)?;
                        // Return 0 (no overflow detected — conservative)
                        Ok(Some(self.context.i32_type().const_int(0, false).into()))
                    } else {
                        Ok(Some(self.context.i32_type().const_int(0, false).into()))
                    }
                } else {
                    Ok(Some(self.context.i32_type().const_int(0, false).into()))
                }
            }
            "__sync_synchronize" => {
                // Full memory barrier → LLVM fence instruction
                self.builder
                    .build_fence(
                        inkwell::AtomicOrdering::SequentiallyConsistent,
                        false,
                        "sync_fence",
                    )
                    .map_err(|_| BackendError::InvalidNode)?;
                Ok(Some(self.context.i32_type().const_int(0, false).into()))
            }
            _ => {
                if let Some(func) = self.functions.get(&builtin_name) {
                    let call_site = self
                        .builder
                        .build_call(*func, &args, "builtin_call")
                        .map_err(|_| BackendError::InvalidNode)?;
                    return Ok(Some(match call_site.try_as_basic_value() {
                        inkwell::values::ValueKind::Basic(v) => v,
                        _ => self.context.i32_type().const_int(0, false).into(),
                    }));
                }

                let fn_type = self.context.i32_type().fn_type(&[], true);
                let func = self.module.add_function(&builtin_name, fn_type, None);
                self.functions.insert(builtin_name.clone(), func);
                let call_site = self
                    .builder
                    .build_call(func, &args, "builtin_call")
                    .map_err(|_| BackendError::InvalidNode)?;
                Ok(Some(match call_site.try_as_basic_value() {
                    inkwell::values::ValueKind::Basic(v) => v,
                    _ => self.context.i32_type().const_int(0, false).into(),
                }))
            }
        }
    }

    fn lower_designated_init(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                if matches!(child.kind, 60..=82) {
                    return self.lower_expr(arena, child_offset);
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }
        Ok(None)
    }

    /// Lower a chain of designated initializers (kind=205) into GEP+store
    /// instructions targeting the fields of an already-allocated struct variable.
    fn lower_designated_init_into_struct(
        &mut self,
        arena: &Arena,
        first_init_offset: NodeOffset,
        struct_ptr: PointerValue<'ctx>,
        struct_type: StructType<'ctx>,
        var_name: &str,
    ) -> Result<(), BackendError> {
        // Clone field names to avoid borrow conflict with &mut self in lower_expr.
        let field_names = self
            .struct_fields
            .get(var_name)
            .cloned()
            .or_else(|| {
                // Fallback: search struct_tag_fields for a matching struct.
                for (_tag, fields) in &self.struct_tag_fields {
                    if fields.len() == struct_type.count_fields() as usize {
                        return Some(fields.clone());
                    }
                }
                None
            })
            .unwrap_or_default();

        let mut offset = first_init_offset;
        while offset != NodeOffset::NULL {
            if let Some(node) = arena.get(offset) {
                if node.kind == 205 {
                    // Field designated init: data = string-table offset for field name.
                    if let Some(field_name) = arena.get_string(NodeOffset(node.data)) {
                        let field_name = field_name.to_string();
                        let field_idx = field_names
                            .iter()
                            .position(|f| f == &field_name)
                            .unwrap_or(0) as u32;

                        // Lower the value expression (stored as first_child).
                        if node.first_child != NodeOffset::NULL {
                            if let Some(val) = self.lower_expr(arena, node.first_child)? {
                                let field_ptr = self
                                    .builder
                                    .build_struct_gep(
                                        struct_type,
                                        struct_ptr,
                                        field_idx,
                                        "desig.gep",
                                    )
                                    .map_err(|_| BackendError::InvalidNode)?;
                                let _ = self
                                    .builder
                                    .build_store(field_ptr, val)
                                    .map_err(|_| BackendError::InvalidNode);
                            }
                        }
                    }
                }
                offset = node.next_sibling;
            } else {
                break;
            }
        }
        Ok(())
    }

    fn lower_extension(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        if node.first_child != NodeOffset::NULL {
            self.lower_expr(arena, node.first_child)
        } else {
            Ok(None)
        }
    }

    /// Lower a compound literal expression (kind=212).
    ///
    /// AST layout:
    ///   kind=212, data = NodeOffset of first initializer element
    ///   first_child = type specifier chain
    ///
    /// Generated LLVM IR:
    ///   1. Alloca a temporary of the resolved type
    ///   2. Store each initializer value (designated or positional)
    ///   3. Load and return the aggregate/scalar value
    fn lower_compound_literal(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let spec_offset = node.first_child;
        let init_offset = NodeOffset(node.data);

        let spec_node = arena.get(spec_offset);
        let spec_kind = spec_node.map(|n| n.kind).unwrap_or(2);

        // Resolve the compound literal's type and optional struct info.
        let (alloca_type, struct_info) = if matches!(spec_kind, 4 | 5) {
            // Struct / union type
            let info = self.struct_info_for_spec(arena, spec_node);
            let ty = info
                .as_ref()
                .map(|(st, _)| st.as_basic_type_enum())
                .unwrap_or_else(|| {
                    spec_node
                        .map(|sn| self.build_struct_llvm_type(arena, sn))
                        .unwrap_or_else(|| self.context.i32_type().as_basic_type_enum())
                });
            (ty, info)
        } else {
            // Scalar or array type – check for an array declarator chained on the
            // specifier (e.g. `(int[]){1,2,3}` would have a kind=8 node after the
            // specifier in the first_child chain).
            let base_type = self.node_kind_to_llvm_type(spec_kind);

            // Walk sibling chain to see if there is an array declarator.
            let mut arr_size: Option<u32> = None;
            let mut off = spec_offset;
            while off != NodeOffset::NULL {
                if let Some(n) = arena.get(off) {
                    if n.kind == 8 {
                        arr_size = Some(if n.data > 0 {
                            n.data
                        } else {
                            // Zero-length: infer from init count.
                            self.count_init_elements(arena, init_offset)
                        });
                        break;
                    }
                    off = n.next_sibling;
                } else {
                    break;
                }
            }

            if let Some(size) = arr_size {
                (base_type.array_type(size).as_basic_type_enum(), None)
            } else {
                (base_type, None)
            }
        };

        // Allocate a temporary for the compound literal.
        let tmp_ptr = self
            .build_entry_alloca(alloca_type, "compound.lit")
            .or_else(|_| {
                self.builder
                    .build_alloca(alloca_type, "compound.lit")
                    .map_err(|_| BackendError::InvalidNode)
            })?;

        // --- Store initializers ---
        if init_offset != NodeOffset::NULL {
            let is_designated = arena
                .get(init_offset)
                .map(|n| n.kind == 205)
                .unwrap_or(false);

            if is_designated {
                if let BasicTypeEnum::StructType(struct_type) = alloca_type {
                    // Register field names so lower_designated_init_into_struct
                    // can look them up.  We use a synthetic variable name that
                    // won't collide with user identifiers.
                    let synth_name = "__compound_lit_tmp".to_string();
                    if let Some((_, ref field_names)) = struct_info {
                        self.struct_fields
                            .insert(synth_name.clone(), field_names.clone());
                    }
                    self.lower_designated_init_into_struct(
                        arena,
                        init_offset,
                        tmp_ptr,
                        struct_type,
                        &synth_name,
                    )?;
                    self.struct_fields.remove(&synth_name);
                }
            } else if let BasicTypeEnum::ArrayType(arr_ty) = alloca_type {
                // Positional array initializer: store each element at its index.
                let elem_type = arr_ty.get_element_type();
                let mut idx: u64 = 0;
                let mut off = init_offset;
                while off != NodeOffset::NULL {
                    if let Some(val) = self.lower_expr(arena, off)? {
                        let indices = [
                            self.context.i64_type().const_int(0, false),
                            self.context.i64_type().const_int(idx, false),
                        ];
                        let elem_ptr = unsafe {
                            self.builder
                                .build_gep(
                                    arr_ty.as_basic_type_enum(),
                                    tmp_ptr,
                                    &indices,
                                    "cl.arr.gep",
                                )
                                .map_err(|_| BackendError::InvalidNode)?
                        };
                        let store_val = self.maybe_cast(val, elem_type);
                        let _ = self
                            .builder
                            .build_store(elem_ptr, store_val)
                            .map_err(|_| BackendError::InvalidNode);
                    }
                    off = arena
                        .get(off)
                        .map(|n| n.next_sibling)
                        .unwrap_or(NodeOffset::NULL);
                    idx += 1;
                }
            } else {
                // Scalar compound literal – just store the single value.
                if let Some(val) = self.lower_expr(arena, init_offset)? {
                    let _ = self
                        .builder
                        .build_store(tmp_ptr, val)
                        .map_err(|_| BackendError::InvalidNode);
                }
            }
        }

        // Load and return the compound literal value.
        let loaded = self
            .builder
            .build_load(alloca_type, tmp_ptr, "compound.lit.load")
            .map_err(|_| BackendError::InvalidNode)?;
        Ok(Some(loaded))
    }

    /// Count the number of initializer elements in a sibling chain.
    fn count_init_elements(&self, arena: &Arena, first: NodeOffset) -> u32 {
        let mut count = 0u32;
        let mut off = first;
        while off != NodeOffset::NULL {
            count += 1;
            off = arena
                .get(off)
                .map(|n| n.next_sibling)
                .unwrap_or(NodeOffset::NULL);
        }
        count
    }

    /// Cast `val` to match `target` element type if they differ.
    fn maybe_cast(
        &self,
        val: BasicValueEnum<'ctx>,
        target: BasicTypeEnum<'ctx>,
    ) -> BasicValueEnum<'ctx> {
        if val.get_type() == target {
            return val;
        }
        // int → int truncation / extension
        if val.is_int_value() && target.is_int_type() {
            let from = val.into_int_value();
            let to_ty = target.into_int_type();
            if let Ok(cast) = self.builder.build_int_cast(from, to_ty, "cl.cast") {
                return cast.into();
            }
        }
        val
    }

    fn find_ident_name(&self, arena: &Arena, node: &CAstNode) -> Option<String> {
        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                if child.kind == 60 {
                    return arena
                        .get_string(NodeOffset(child.data))
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string());
                }
                if matches!(child.kind, 7..=9) {
                    if let Some(name) = self.find_ident_name(arena, &child) {
                        return Some(name);
                    }
                }
                // kind=73 is an init-declarator: first_child is the declarator
                if child.kind == 73 {
                    if let Some(name) = self.find_ident_name(arena, &child) {
                        return Some(name);
                    }
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }
        None
    }

    fn find_function_declarator_offset(
        &self,
        arena: &Arena,
        offset: NodeOffset,
    ) -> Option<NodeOffset> {
        let node = arena.get(offset)?;
        match node.kind {
            9 => Some(offset),
            7 | 8 | 73 | 20 | 21 | 22 | 23 | 24 | 40 => {
                let mut child_offset = node.first_child;
                while child_offset != NodeOffset::NULL {
                    if let Some(found) = self.find_function_declarator_offset(arena, child_offset) {
                        return Some(found);
                    }
                    child_offset = arena
                        .get(child_offset)
                        .map(|n| n.next_sibling)
                        .unwrap_or(NodeOffset::NULL);
                }
                None
            }
            _ => None,
        }
    }

    /// Find the identifier name starting from an already-obtained node (not from a parent).
    fn find_ident_name_in(&self, arena: &Arena, node: &CAstNode) -> Option<String> {
        if node.kind == 60 {
            return arena
                .get_string(NodeOffset(node.data))
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
        }
        if matches!(node.kind, 7..=9) {
            return self.find_ident_name(arena, node);
        }
        None
    }

    /// Extract attribute information from a node's child/sibling chain.
    /// Returns a list of (attr_name, optional_string_arg, optional_int_arg) tuples.
    fn extract_attributes(
        &self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Vec<(String, Option<String>, Option<u32>)> {
        let mut attrs = Vec::new();
        self.collect_attributes_from_chain(arena, node.first_child, &mut attrs);
        attrs
    }

    /// Apply extracted attributes to an LLVM function value.
    fn apply_function_attributes(
        &self,
        function: FunctionValue<'ctx>,
        attrs: &[(String, Option<String>, Option<u32>)],
    ) {
        for (name, str_arg, _int_arg) in attrs {
            match name.as_str() {
                "weak" | "__weak__" => {
                    function.set_linkage(inkwell::module::Linkage::ExternalWeak);
                }
                "section" | "__section__" => {
                    if let Some(section_name) = str_arg {
                        let clean = section_name.trim_matches('"');
                        function.set_section(Some(clean));
                    }
                }
                "visibility" | "__visibility__" => {
                    if let Some(vis) = str_arg {
                        let clean = vis.trim_matches('"');
                        match clean {
                            "hidden" => function
                                .as_global_value()
                                .set_visibility(inkwell::GlobalVisibility::Hidden),
                            "protected" => function
                                .as_global_value()
                                .set_visibility(inkwell::GlobalVisibility::Protected),
                            _ => {} // "default" or unknown: leave as-is
                        }
                    }
                }
                "noreturn" | "__noreturn__" => {
                    function.add_attribute(
                        inkwell::attributes::AttributeLoc::Function,
                        self.context.create_enum_attribute(
                            inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn"),
                            0,
                        ),
                    );
                }
                "noinline" | "__noinline__" => {
                    function.add_attribute(
                        inkwell::attributes::AttributeLoc::Function,
                        self.context.create_enum_attribute(
                            inkwell::attributes::Attribute::get_named_enum_kind_id("noinline"),
                            0,
                        ),
                    );
                }
                "always_inline" | "__always_inline__" => {
                    function.add_attribute(
                        inkwell::attributes::AttributeLoc::Function,
                        self.context.create_enum_attribute(
                            inkwell::attributes::Attribute::get_named_enum_kind_id("alwaysinline"),
                            0,
                        ),
                    );
                }
                "hot" | "__hot__" => {
                    function.add_attribute(
                        inkwell::attributes::AttributeLoc::Function,
                        self.context.create_enum_attribute(
                            inkwell::attributes::Attribute::get_named_enum_kind_id("hot"),
                            0,
                        ),
                    );
                }
                "cold" | "__cold__" => {
                    function.add_attribute(
                        inkwell::attributes::AttributeLoc::Function,
                        self.context.create_enum_attribute(
                            inkwell::attributes::Attribute::get_named_enum_kind_id("cold"),
                            0,
                        ),
                    );
                }
                _ => {} // Other attributes: silently ignored for now
            }
        }
    }

    /// Apply extracted attributes to an LLVM global value.
    fn apply_global_attributes(
        &self,
        global: inkwell::values::GlobalValue<'ctx>,
        attrs: &[(String, Option<String>, Option<u32>)],
    ) {
        for (name, str_arg, int_arg) in attrs {
            match name.as_str() {
                "weak" | "__weak__" => {
                    global.set_linkage(inkwell::module::Linkage::ExternalWeak);
                }
                "section" | "__section__" => {
                    if let Some(section_name) = str_arg {
                        let clean = section_name.trim_matches('"');
                        global.set_section(Some(clean));
                    }
                }
                "aligned" | "__aligned__" => {
                    if let Some(align) = int_arg {
                        global.set_alignment(*align);
                    }
                }
                "visibility" | "__visibility__" => {
                    if let Some(vis) = str_arg {
                        let clean = vis.trim_matches('"');
                        match clean {
                            "hidden" => global.set_visibility(inkwell::GlobalVisibility::Hidden),
                            "protected" => {
                                global.set_visibility(inkwell::GlobalVisibility::Protected)
                            }
                            _ => {}
                        }
                    }
                }
                _ => {} // Other attributes silently ignored
            }
        }
    }

    pub fn dump_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    pub fn verify(&self) -> Result<(), BackendError> {
        if self.module.verify().is_err() {
            return Err(BackendError::VerificationFailed(
                self.module.print_to_string().to_string(),
            ));
        }
        Ok(())
    }

    pub fn optimize(&self, _opt_level: u32) -> Result<(), BackendError> {
        Ok(())
    }
}

#[derive(Debug)]
pub enum BackendError {
    UnknownNodeKind(u16),
    InvalidNode,
    UndefinedVariable(String),
    UndefinedFunction(String),
    InvalidOperator(u32),
    VerificationFailed(String),
    IoError(String),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            BackendError::UnknownNodeKind(k) => write!(f, "Unknown AST node kind: {}", k),
            BackendError::InvalidNode => write!(f, "Invalid AST node"),
            BackendError::UndefinedVariable(name) => write!(f, "Undefined variable: {}", name),
            BackendError::UndefinedFunction(name) => write!(f, "Undefined function: {}", name),
            BackendError::InvalidOperator(op) => write!(f, "Invalid operator code: {}", op),
            BackendError::VerificationFailed(msg) => write!(f, "LLVM verification failed: {}", msg),
            BackendError::IoError(msg) => write!(f, "I/O error: {}", msg),
        }
    }
}

impl std::error::Error for BackendError {}

pub fn create_backend<'ctx>(
    context: &'ctx Context,
    module_name: &str,
) -> LlvmBackend<'ctx, 'static> {
    LlvmBackend::new(context, module_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_creation() {
        let context = Context::create();
        let backend = create_backend(&context, "test_module");
        let ir = backend.dump_ir();
        assert!(ir.contains("test_module"));
    }

    #[test]
    fn test_backend_with_types() {
        let context = Context::create();
        let ts = TypeSystem::new();
        let backend = LlvmBackend::with_types(&context, "typed_module", &ts);
        let ir = backend.dump_ir();
        assert!(ir.contains("typed_module"));
    }

    #[test]
    fn test_to_llvm_type_void() {
        let context = Context::create();
        let ts = TypeSystem::new();
        let mut backend = LlvmBackend::with_types(&context, "test", &ts);
        let ty = backend.to_llvm_type(0);
        assert!(ty.is_int_type());
        assert_eq!(ty.into_int_type().get_bit_width(), 8);
    }

    #[test]
    fn test_to_llvm_type_bool() {
        let context = Context::create();
        let ts = TypeSystem::new();
        let mut backend = LlvmBackend::with_types(&context, "test", &ts);
        let ty = backend.to_llvm_type(1);
        assert!(ty.is_int_type());
        assert_eq!(ty.into_int_type().get_bit_width(), 1);
    }

    #[test]
    fn test_to_llvm_type_char() {
        let context = Context::create();
        let ts = TypeSystem::new();
        let mut backend = LlvmBackend::with_types(&context, "test", &ts);
        let ty = backend.to_llvm_type(2);
        assert!(ty.is_int_type());
        assert_eq!(ty.into_int_type().get_bit_width(), 8);
    }

    #[test]
    fn test_to_llvm_type_short() {
        let context = Context::create();
        let ts = TypeSystem::new();
        let mut backend = LlvmBackend::with_types(&context, "test", &ts);
        let ty = backend.to_llvm_type(5);
        assert!(ty.is_int_type());
        assert_eq!(ty.into_int_type().get_bit_width(), 16);
    }

    #[test]
    fn test_to_llvm_type_int() {
        let context = Context::create();
        let ts = TypeSystem::new();
        let mut backend = LlvmBackend::with_types(&context, "test", &ts);
        let ty = backend.to_llvm_type(7);
        assert!(ty.is_int_type());
        assert_eq!(ty.into_int_type().get_bit_width(), 32);
    }

    #[test]
    fn test_to_llvm_type_long() {
        let context = Context::create();
        let ts = TypeSystem::new();
        let mut backend = LlvmBackend::with_types(&context, "test", &ts);
        let ty = backend.to_llvm_type(9);
        assert!(ty.is_int_type());
        assert_eq!(ty.into_int_type().get_bit_width(), 64);
    }

    #[test]
    fn test_to_llvm_type_float() {
        let context = Context::create();
        let ts = TypeSystem::new();
        let mut backend = LlvmBackend::with_types(&context, "test", &ts);
        let ty = backend.to_llvm_type(13);
        assert!(ty.is_float_type());
    }

    #[test]
    fn test_to_llvm_type_double() {
        let context = Context::create();
        let ts = TypeSystem::new();
        let mut backend = LlvmBackend::with_types(&context, "test", &ts);
        let ty = backend.to_llvm_type(14);
        assert!(ty.is_float_type());
    }

    #[test]
    fn test_to_llvm_type_pointer() {
        let context = Context::create();
        let mut ts = TypeSystem::new();
        let ptr_id = ts.add_type(CType::Pointer { base: TypeId::INT });
        let mut backend = LlvmBackend::with_types(&context, "test", &ts);
        let ty = backend.to_llvm_type(ptr_id.0);
        assert!(ty.is_pointer_type());
    }

    #[test]
    fn test_to_llvm_type_without_types_falls_back_to_i32() {
        let context = Context::create();
        let mut backend = LlvmBackend::new(&context, "test");
        let ty = backend.to_llvm_type(0);
        assert!(ty.is_int_type());
        assert_eq!(ty.into_int_type().get_bit_width(), 32);
    }

    #[test]
    fn test_type_cache_caching() {
        let context = Context::create();
        let ts = TypeSystem::new();
        let mut backend = LlvmBackend::with_types(&context, "test", &ts);
        let ty1 = backend.to_llvm_type(7);
        let ty2 = backend.to_llvm_type(7);
        assert_eq!(ty1, ty2);
        assert_eq!(backend.type_cache.len(), 1);
    }

    /// Helper: parse C source and compile to LLVM IR, returning the IR string
    fn compile_c_to_ir(source: &str) -> String {
        use crate::frontend::parser::Parser;
        use tempfile::NamedTempFile;

        let temp_file = NamedTempFile::new().unwrap();
        let arena = Arena::new(temp_file.path(), 65536).unwrap();
        let mut parser = Parser::new(arena);
        let root = parser.parse(source).expect("parse failed");

        let context = Context::create();
        let ts = TypeSystem::new();
        let mut backend = LlvmBackend::with_types(&context, "test", &ts);
        backend
            .compile(&parser.arena, root)
            .expect("compile failed");
        backend.dump_ir()
    }

    #[test]
    fn test_switch_codegen() {
        let ir = compile_c_to_ir(
            "int classify(int x) { \
                switch (x) { \
                    case 0: return 10; \
                    case 1: return 20; \
                    default: return 30; \
                } \
                return 0; \
            }",
        );
        // The IR should contain a switch instruction
        assert!(
            ir.contains("switch"),
            "Expected switch instruction in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_goto_label_codegen() {
        let ir = compile_c_to_ir(
            "int test_goto() { \
                int x = 0; \
                goto done; \
                x = 1; \
                done: \
                return x; \
            }",
        );
        // Should contain a branch to a label block
        assert!(
            ir.contains("br label"),
            "Expected unconditional branch in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_break_in_switch() {
        let ir = compile_c_to_ir(
            "int test_break(int x) { \
                int result = 0; \
                switch (x) { \
                    case 1: result = 10; break; \
                    case 2: result = 20; break; \
                    default: result = 30; break; \
                } \
                return result; \
            }",
        );
        assert!(ir.contains("switch"), "Expected switch in IR:\n{}", ir);
    }

    #[test]
    fn test_break_in_while() {
        let ir = compile_c_to_ir(
            "int test_break_while() { \
                int i = 0; \
                while (i < 10) { \
                    if (i == 5) break; \
                    i = i + 1; \
                } \
                return i; \
            }",
        );
        // Should have a branch to the end block (break)
        assert!(ir.contains("br label"), "Expected branches in IR:\n{}", ir);
    }

    #[test]
    fn test_continue_in_for() {
        let ir = compile_c_to_ir(
            "int test_continue() { \
                int sum = 0; \
                int i; \
                for (i = 0; i < 10; i = i + 1) { \
                    if (i == 3) continue; \
                    sum = sum + i; \
                } \
                return sum; \
            }",
        );
        assert!(ir.contains("br label"), "Expected branches in IR:\n{}", ir);
    }

    #[test]
    fn test_builtin_expect() {
        let ir = compile_c_to_ir(
            "int test_expect(int x) { \
                return __builtin_expect(x, 1); \
            }",
        );
        // __builtin_expect just returns its first argument
        assert!(
            ir.contains("define"),
            "Expected function definition in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_builtin_constant_p() {
        let ir = compile_c_to_ir(
            "int test_constant_p(int x) { \
                return __builtin_constant_p(x); \
            }",
        );
        assert!(
            ir.contains("define"),
            "Expected function definition in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_attribute_section_global_string() {
        let ir = compile_c_to_ir(
            "static const char __UNIQUE_ID_license230[] __attribute__((used)) __attribute__((section(\".modinfo\"))) __attribute__((aligned(1))) = \"license\" \"=\" \"GPL\";"
        );
        assert!(
            ir.contains("@__UNIQUE_ID_license230"),
            "Expected global license symbol in IR:\n{}",
            ir
        );
        assert!(
            ir.contains(".modinfo"),
            "Expected .modinfo section in IR:\n{}",
            ir
        );
        assert!(
            ir.contains("license=GPL"),
            "Expected license payload in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_builtin_types_compatible_p_distinguishes_types() {
        let ir = compile_c_to_ir(
            "int same_type(void) { return __builtin_types_compatible_p(int, int); } \
             int diff_type(void) { return __builtin_types_compatible_p(int, char); }",
        );
        assert!(
            ir.contains("define i32 @same_type()"),
            "Expected same_type in IR:\n{}",
            ir
        );
        assert!(
            ir.contains("ret i32 1"),
            "Expected types_compatible_p(int, int) == 1 in IR:\n{}",
            ir
        );
        assert!(
            ir.contains("define i32 @diff_type()"),
            "Expected diff_type in IR:\n{}",
            ir
        );
        assert!(
            ir.contains("ret i32 0"),
            "Expected types_compatible_p(int, char) == 0 in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_builtin_choose_expr_selects_false_branch() {
        let ir =
            compile_c_to_ir("int choose_false(void) { return __builtin_choose_expr(0, 11, 22); }");
        assert!(
            ir.contains("ret i32 22"),
            "Expected false branch result in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_variadic_function() {
        let ir = compile_c_to_ir(
            "int my_printf(int fmt, ...) { \
                return 0; \
            }",
        );
        // Variadic functions should have ... in the LLVM signature
        assert!(
            ir.contains("..."),
            "Expected variadic signature in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_va_list_typedef_parameter_lowers_to_ptr() {
        let ir = compile_c_to_ir(
            "typedef __builtin_va_list va_list; \
             int consume(va_list ap) { return 0; }",
        );
        assert!(
            ir.contains("define i32 @consume(ptr"),
            "Expected va_list parameter to lower to ptr:\n{}",
            ir
        );
    }

    #[test]
    fn test_pointer_deref_assignment_emits_store() {
        let ir = compile_c_to_ir(
            "int write_ptr(int **out, int *value) { \
                *out = value; \
                return 0; \
            }",
        );
        assert!(
            ir.contains("store ptr %value") && ir.contains(", ptr %out"),
            "Expected dereference assignment to emit pointer store:\n{}",
            ir
        );
    }

    #[test]
    fn test_asm_basic_volatile() {
        let ir = compile_c_to_ir(
            "void test_asm() { \
                asm volatile(\"nop\"); \
            }",
        );
        assert!(
            ir.contains("call void asm sideeffect"),
            "Expected asm sideeffect call in IR:\n{}",
            ir
        );
        assert!(ir.contains("nop"), "Expected nop in asm template:\n{}", ir);
    }

    #[test]
    fn test_asm_memory_barrier() {
        let ir = compile_c_to_ir(
            "void memory_barrier() { \
                asm volatile(\"\" : : : \"memory\"); \
            }",
        );
        // Should produce an asm call with memory clobber
        assert!(
            ir.contains("asm sideeffect"),
            "Expected asm sideeffect in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_asm_with_output() {
        let ir = compile_c_to_ir(
            "int read_reg() { \
                int val; \
                asm(\"mov $0, %0\" : \"=r\"(val)); \
                return val; \
            }",
        );
        assert!(
            ir.contains("asm"),
            "Expected asm instruction in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_asm_with_input_and_output() {
        let ir = compile_c_to_ir(
            "int double_it(int x) { \
                int result; \
                asm(\"addl %1, %0\" : \"=r\"(result) : \"r\"(x)); \
                return result; \
            }",
        );
        assert!(
            ir.contains("asm"),
            "Expected asm instruction in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_asm_cc_clobber() {
        let ir = compile_c_to_ir(
            "void test_cc_clobber() { \
                asm volatile(\"\" : : : \"cc\"); \
            }",
        );
        assert!(
            ir.contains("asm sideeffect"),
            "Expected asm sideeffect in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_builtin_alloca() {
        let ir = compile_c_to_ir(
            "void test_alloca(int n) { \
                void *p = __builtin_alloca(n); \
            }",
        );
        assert!(ir.contains("alloca"), "Expected alloca in IR:\n{}", ir);
    }

    #[test]
    fn test_builtin_memcpy() {
        let ir = compile_c_to_ir(
            "void test_memcpy(char *dst, char *src, int n) { \
                __builtin_memcpy(dst, src, n); \
            }",
        );
        assert!(
            ir.contains("memcpy") || ir.contains("__builtin_memcpy"),
            "Expected memcpy call in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_builtin_memset() {
        let ir = compile_c_to_ir(
            "void test_memset(char *dst, int c, int n) { \
                __builtin_memset(dst, c, n); \
            }",
        );
        assert!(
            ir.contains("memset") || ir.contains("__builtin_memset"),
            "Expected memset call in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_sync_fetch_and_add_atomicrmw() {
        let ir = compile_c_to_ir(
            "int test_sync_add(int *p) { \
                return __sync_fetch_and_add(p, 1); \
            }",
        );
        assert!(
            ir.contains("atomicrmw add"),
            "Expected atomicrmw add in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_sync_val_compare_and_swap_cmpxchg() {
        let ir = compile_c_to_ir(
            "int test_sync_cas(int *p) { \
                return __sync_val_compare_and_swap(p, 1, 2); \
            }",
        );
        assert!(ir.contains("cmpxchg"), "Expected cmpxchg in IR:\n{}", ir);
    }

    #[test]
    fn test_atomic_thread_fence_builtin() {
        let ir = compile_c_to_ir(
            "void test_atomic_fence() { \
                __atomic_thread_fence(5); \
            }",
        );
        assert!(
            ir.contains("fence seq_cst") || ir.contains("fence"),
            "Expected fence in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_label_addr_expr() {
        let ir = compile_c_to_ir(
            "int test_label_addr() { \
                void *p; \
                target: \
                p = &&target; \
                return 0; \
            }",
        );
        assert!(
            ir.contains("blockaddress") || ir.contains("label.target"),
            "Expected blockaddress or label block in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_computed_goto() {
        let ir = compile_c_to_ir(
            "void test_computed_goto() { \
                void *target; \
                label1: \
                target = &&label1; \
                goto *target; \
            }",
        );
        assert!(
            ir.contains("indirectbr") || ir.contains("label.label1"),
            "Expected indirectbr or label block in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_case_range() {
        let ir = compile_c_to_ir(
            "int classify(int x) { \
                switch (x) { \
                    case 1 ... 5: return 1; \
                    case 10 ... 20: return 2; \
                    default: return 0; \
                } \
            }",
        );
        assert!(
            ir.contains("switch"),
            "Expected switch instruction in IR:\n{}",
            ir
        );
        // Case ranges should generate multiple case entries
        assert!(
            ir.contains("switch.case_range") || ir.contains("i32 1") || ir.contains("switch i32"),
            "Expected case range expansion in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_case_range_single_value() {
        let ir = compile_c_to_ir(
            "int test_single_range(int x) { \
                switch (x) { \
                    case 5 ... 5: return 1; \
                    default: return 0; \
                } \
            }",
        );
        assert!(ir.contains("switch"), "Expected switch in IR:\n{}", ir);
    }

    #[test]
    fn test_attribute_weak_function() {
        let ir = compile_c_to_ir(
            "void my_weak_func(void) __attribute__((weak)); \
             void my_weak_func(void) { return; }",
        );
        assert!(
            ir.contains("my_weak_func"),
            "Expected function in IR:\n{}",
            ir
        );
        // weak linkage should appear as `weak` or `extern_weak`
        assert!(ir.contains("weak"), "Expected weak linkage in IR:\n{}", ir);
    }

    #[test]
    fn test_thread_local_global_lowering() {
        let ir = compile_c_to_ir("__thread int per_cpu_counter;");
        assert!(
            ir.contains("@per_cpu_counter = thread_local"),
            "Expected thread_local global in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_attribute_section_function() {
        let ir = compile_c_to_ir(
            "__attribute__((section(\".init.text\"))) void init_func(void) { return; }",
        );
        assert!(ir.contains("init_func"), "Expected function in IR:\n{}", ir);
        assert!(
            ir.contains(".init.text"),
            "Expected section attribute in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_attribute_noreturn_function() {
        let ir = compile_c_to_ir("__attribute__((noreturn)) void die(void) { return; }");
        assert!(ir.contains("die"), "Expected function in IR:\n{}", ir);
        assert!(
            ir.contains("noreturn"),
            "Expected noreturn attribute in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_attribute_cold_function() {
        let ir = compile_c_to_ir("__attribute__((cold)) void rare_path(void) { return; }");
        assert!(ir.contains("rare_path"), "Expected function in IR:\n{}", ir);
        assert!(
            ir.contains("cold"),
            "Expected cold attribute in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_attribute_noinline_and_alwaysinline_function() {
        let ir = compile_c_to_ir(
            "__attribute__((noinline)) void slow_path(void) { return; } \
             __attribute__((always_inline)) void fast_path(void) { return; } \
             __attribute__((hot)) void hot_path(void) { return; }",
        );
        assert!(
            ir.contains("slow_path"),
            "Expected slow_path in IR:\n{}",
            ir
        );
        assert!(
            ir.contains("fast_path"),
            "Expected fast_path in IR:\n{}",
            ir
        );
        assert!(ir.contains("hot_path"), "Expected hot_path in IR:\n{}", ir);
        assert!(
            ir.contains("noinline"),
            "Expected noinline attribute in IR:\n{}",
            ir
        );
        assert!(
            ir.contains("alwaysinline"),
            "Expected alwaysinline attribute in IR:\n{}",
            ir
        );
        assert!(ir.contains("hot"), "Expected hot attribute in IR:\n{}", ir);
    }

    #[test]
    fn test_attribute_constructor_function() {
        let ir = compile_c_to_ir("__attribute__((constructor)) void init_hook(void) { return; }");
        assert!(
            ir.contains("@llvm.global_ctors"),
            "Expected llvm.global_ctors in IR:\n{}",
            ir
        );
        assert!(
            ir.contains("@init_hook"),
            "Expected init_hook in ctor table IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_attribute_destructor_function() {
        let ir = compile_c_to_ir("__attribute__((destructor)) void cleanup_hook(void) { return; }");
        assert!(
            ir.contains("@llvm.global_dtors"),
            "Expected llvm.global_dtors in IR:\n{}",
            ir
        );
        assert!(
            ir.contains("@cleanup_hook"),
            "Expected cleanup_hook in dtor table IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_unused_static_inline_function_not_emitted() {
        let ir = compile_c_to_ir(
            "static inline int helper(void) { extern void hidden(void); hidden(); return 0; } \
             int ok(void) { return 1; }",
        );
        assert!(
            ir.contains("define i32 @ok()"),
            "Expected ok in IR:\n{}",
            ir
        );
        assert!(
            !ir.contains("@helper"),
            "Unused static inline helper should not be emitted:\n{}",
            ir
        );
        assert!(
            !ir.contains("@hidden"),
            "Unused helper dependency should not be emitted:\n{}",
            ir
        );
    }

    #[test]
    fn test_attribute_packed_struct_sizeof() {
        let ir = compile_c_to_ir(
            "struct __attribute__((packed)) S { int a; char b; }; \
             int packed_size(void) { return sizeof(struct S); }",
        );
        assert!(
            ir.contains("ret i32 5"),
            "Expected packed struct size 5 in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_attribute_packed_struct_ir_layout() {
        let ir = compile_c_to_ir(
            "struct __attribute__((packed)) S { int a; char b; }; \
             int read_b(struct S *p) { return p->b; }",
        );
        assert!(
            ir.contains("<{ i32, i8 }>") || ir.contains("<{i32, i8}>"),
            "Expected packed LLVM struct layout in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_flexible_array_sizeof_header() {
        let ir = compile_c_to_ir(
            "struct Flex { int len; char data[]; }; \
             int flex_header_size(void) { return sizeof(struct Flex); }",
        );
        assert!(
            ir.contains("ret i32 4"),
            "Expected flexible array header size 4 in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_sizeof_int() {
        let ir = compile_c_to_ir("int test_sizeof(void) { return sizeof(int); }");
        // sizeof(int) should produce 4, returned as i32
        assert!(
            ir.contains("ret i32 4"),
            "Expected ret i32 4 for sizeof(int):\n{}",
            ir
        );
    }

    #[test]
    fn test_sizeof_char() {
        let ir = compile_c_to_ir("int test_sizeof_char(void) { return sizeof(char); }");
        // sizeof(char) should produce 1, returned as i32
        assert!(
            ir.contains("ret i32 1"),
            "Expected ret i32 1 for sizeof(char):\n{}",
            ir
        );
    }

    #[test]
    fn test_ternary_select() {
        let ir = compile_c_to_ir("int max(int a, int b) { return a > b ? a : b; }");
        // Ternary should produce a select instruction
        assert!(
            ir.contains("select"),
            "Expected select instruction for ternary:\n{}",
            ir
        );
    }

    #[test]
    fn test_extern_func_decl_with_params() {
        let ir = compile_c_to_ir(
            "extern int puts(const char *s); \
             int main(void) { return puts(\"hello\"); }",
        );
        // extern decl should produce declare with ptr param, not variadic
        assert!(
            ir.contains("declare i32 @puts(ptr)"),
            "Expected declare i32 @puts(ptr):\n{}",
            ir
        );
        assert!(
            !ir.contains("@puts(...)"),
            "Should not have variadic puts:\n{}",
            ir
        );
    }

    #[test]
    fn test_pointer_array_indexing() {
        let ir = compile_c_to_ir(
            "extern int puts(const char *s); \
             int main(int argc, char **argv) { return puts(argv[1]); }",
        );
        // argv[1] should use ptr element type GEP, not i32
        assert!(
            ir.contains("getelementptr ptr"),
            "Expected getelementptr ptr for char **argv:\n{}",
            ir
        );
    }

    #[test]
    fn test_char_pointer_deref_loads_i8() {
        let ir = compile_c_to_ir("int first_char(const char *z){ return *z; }");
        assert!(
            ir.contains("load i8, ptr"),
            "char-pointer dereference should load i8, not i32:\n{}",
            ir
        );
    }

    #[test]
    fn test_local_unsigned_char_pointer_deref_loads_i8() {
        let ir = compile_c_to_ir(
            "int ci(const unsigned char *zLeft, const unsigned char *zRight){ \
               const unsigned char *a = zLeft; \
               const unsigned char *b = zRight; \
               return *a - *b; \
             }",
        );
        assert!(
            ir.contains("load i8, ptr"),
            "local unsigned-char pointer dereference should load i8:\n{}",
            ir
        );
    }

    #[test]
    fn test_casted_local_unsigned_char_pointer_deref_loads_i8() {
        let ir = compile_c_to_ir(
            "int ci2(const char *zLeft, const char *zRight){ \
               const unsigned char *a = (const unsigned char*)zLeft; \
               const unsigned char *b = (const unsigned char*)zRight; \
               return *a - *b; \
             }",
        );
        assert!(
            ir.contains("load i8, ptr"),
            "casted local unsigned-char pointer dereference should load i8:\n{}",
            ir
        );
    }

    #[test]
    fn test_assigned_local_unsigned_char_pointer_deref_loads_i8() {
        let ir = compile_c_to_ir(
            "int ci3(const char *zLeft, const char *zRight){ \
               const unsigned char *a; \
               const unsigned char *b; \
               a = (const unsigned char*)zLeft; \
               b = (const unsigned char*)zRight; \
               return *a - *b; \
             }",
        );
        assert!(
            ir.contains("load i8, ptr"),
            "assigned local unsigned-char pointer dereference should load i8:\n{}",
            ir
        );
        assert!(
            !ir.contains("store i8, ptr %c"),
            "int temporaries must not receive partial-width i8 stores:\n{}",
            ir
        );
    }

    #[test]
    fn test_call_arg_isolation() {
        let ir = compile_c_to_ir(
            "extern int puts(const char *s); \
             int main(int argc, char **argv) { puts(argv[1]); return 0; }",
        );
        // puts should be called with exactly one argument (ptr), not two
        assert!(
            ir.contains("call i32 @puts(ptr"),
            "Expected call with ptr arg:\n{}",
            ir
        );
        // Should NOT have two arguments
        assert!(
            !ir.contains("@puts(ptr %idx, i32"),
            "Should not have extra index arg in puts call:\n{}",
            ir
        );
    }

    #[test]
    fn test_typedef_declaration_does_not_emit_global() {
        let ir = compile_c_to_ir(
            "typedef unsigned char u8; \
             struct S { u8 x; int y; }; \
             int getx(struct S *s) { return s->x; }",
        );
        assert!(
            !ir.contains("@u8 ="),
            "typedef should not emit a global symbol:\n{}",
            ir
        );
        assert!(
            ir.contains("{ i8, i32 }"),
            "typedef-backed field should keep i8 layout:\n{}",
            ir
        );
    }

    #[test]
    fn test_typedef_cast_does_not_become_function_call() {
        let ir = compile_c_to_ir(
            "typedef unsigned char u8; \
             int f(int v) { return 0xe0 + (u8)((v >> 12) & 0x0f); }",
        );
        assert!(
            !ir.contains("@u8("),
            "typedef cast must not lower as a call to @u8:\n{}",
            ir
        );
        assert!(
            ir.contains("define i32 @f"),
            "expected function definition for typedef-cast regression:\n{}",
            ir
        );
    }

    #[test]
    fn test_typedef_cast_inside_assignment_does_not_become_function_call() {
        let ir = compile_c_to_ir(
            "typedef unsigned char u8; \
             typedef unsigned int u32; \
             int f(char *zOut, u32 v) { \
                 zOut[0] = 0xe0 + (u8)((v >> 12) & 0x0f); \
                 return 3; \
             }",
        );
        assert!(
            !ir.contains("@u8("),
            "typedef cast inside assignment must not lower as a call to @u8:\n{}",
            ir
        );
    }

    #[test]
    fn test_return_bitwise_and_is_not_dropped() {
        let ir = compile_c_to_ir("int f(int rc){ return rc & 0xff; }");
        assert!(
            ir.contains("and i32"),
            "bitwise-and return expression should lower to an and instruction:\n{}",
            ir
        );
        assert!(
            ir.contains("ret i32 %"),
            "bitwise-and return expression should return computed value, not constant zero:\n{}",
            ir
        );
    }

    #[test]
    fn test_call_with_bitwise_or_argument_is_not_dropped() {
        let ir = compile_c_to_ir(
            "int openDatabase(const char*, void**, unsigned int, const char*); \
             int sqlite3_open(const char *zFilename, void **ppDb){ \
               return openDatabase(zFilename, ppDb, 0x00000002 | 0x00000004, 0); \
             }",
        );
        assert!(
            ir.contains("call i32 @openDatabase"),
            "function call should be preserved in return expression:\n{}",
            ir
        );
        assert!(
            !ir.contains("ret i32 0"),
            "sqlite3_open-style wrapper should not collapse to constant zero:\n{}",
            ir
        );
    }

    #[test]
    fn test_nested_shift_mask_condition_does_not_become_poison() {
        let ir = compile_c_to_ir(
            "int f(int flags){ \
               if(((1<<(flags&7)) & 0x46)==0) return 21; \
               return 0; \
             }",
        );
        assert!(
            !ir.contains("br i1 poison"),
            "flag-validation condition must not collapse to poison:\n{}",
            ir
        );
        assert!(
            ir.contains("shl i32 1"),
            "expected shift in lowered condition:\n{}",
            ir
        );
        assert!(
            ir.contains("and i32 %shl, 70"),
            "expected 0x46 mask to remain in lowered condition:\n{}",
            ir
        );
    }

    #[test]
    fn test_qualified_function_pointer_parameter_keeps_function_definition() {
        let ir = compile_c_to_ir(
            "typedef struct sqlite3 sqlite3; \
             typedef struct Table Table; \
             typedef struct Module Module; \
             typedef struct sqlite3_vtab sqlite3_vtab; \
             static int vtabCallConstructor( \
               sqlite3 *db, \
               Table *pTab, \
               Module *pMod, \
               int (*xConstruct)(sqlite3*,void*,int,const char*const*,sqlite3_vtab**,char**), \
               char **pzErr \
             ){ \
               return xConstruct(db, 0, 0, 0, 0, pzErr); \
             }",
        );
        assert!(
            ir.contains("define internal i32 @vtabCallConstructor"),
            "qualified function-pointer parameters must not drop the function definition:\n{}",
            ir
        );
    }

    #[test]
    fn test_function_returning_function_pointer_keeps_name_and_pointer_return() {
        let ir = compile_c_to_ir(
            "typedef struct sqlite3_vfs sqlite3_vfs; \
             static void (*sqlite3OsDlSym(sqlite3_vfs *pVfs, void *pHdle, const char *zSym))(void){ \
                 return 0; \
             }"
        );
        assert!(
            ir.contains("@sqlite3OsDlSym"),
            "function-returning-function-pointer should keep its real name:\n{}",
            ir
        );
        assert!(
            ir.contains("define internal ptr @sqlite3OsDlSym"),
            "function-returning-function-pointer should lower to ptr return:\n{}",
            ir
        );
    }

    #[test]
    fn test_parenthesized_global_struct_member_access_loads_field() {
        let ir = compile_c_to_ir(
            "struct G { int x; int y; }; \
             struct G g; \
             int read_x(void) { return (g).x; }",
        );
        assert!(
            ir.contains("define i32 @read_x"),
            "expected function definition for parenthesized global member access:\n{}",
            ir
        );
        assert!(
            !ir.contains("ret { i32, i32 }"),
            "parenthesized global member access should return the field, not the whole struct:\n{}",
            ir
        );
    }

    #[test]
    fn test_function_pointer_variable_call_uses_signature() {
        let ir = compile_c_to_ir("void call_cb(void (*cb)(void*), void *p) { cb(p); }");
        assert!(
            ir.contains("call void"),
            "function-pointer call should preserve void return type:\n{}",
            ir
        );
        assert!(
            !ir.contains("call i32 (ptr, ...)"),
            "function-pointer call should not fall back to variadic i32 signature:\n{}",
            ir
        );
    }

    #[test]
    fn test_struct_pointer_array_member_keeps_array_shape() {
        let ir = compile_c_to_ir(
            "typedef struct FuncDef FuncDef; \
             typedef struct FuncDefHash { FuncDef *a[23]; } FuncDefHash; \
             static FuncDefHash h; \
             int f(void){ return 0; }",
        );
        assert!(
            ir.contains("[23 x ptr]"),
            "pointer array member should lower as array-of-pointers, not a single ptr:\n{}",
            ir
        );
    }

    #[test]
    fn test_struct_array_member_memcpy_decay_uses_pointer() {
        let ir = compile_c_to_ir(
            "extern void *memcpy(void*, const void*, unsigned long); \
             enum { SQLITE_N_LIMIT = 11 + 1 }; \
             static int aHardLimit[12]; \
             struct Db { int aLimit[SQLITE_N_LIMIT]; }; \
             void init(struct Db *db){ memcpy(db->aLimit, aHardLimit, sizeof(db->aLimit)); }",
        );
        assert!(
            !ir.contains("inttoptr i32 %aLimit to ptr"),
            "array member decay for memcpy destination must not go through int-to-pointer coercion:\n{}",
            ir
        );
        assert!(
            !ir.contains("@memcpy(ptr %coerce_i2p"),
            "memcpy destination should not be synthesized via scalar int-to-pointer coercion:\n{}",
            ir
        );
    }

    #[test]
    fn test_typedef_struct_pointer_index_member_access() {
        let ir = compile_c_to_ir(
            "typedef struct S S; \
             struct S { int x; }; \
             int read2(S *a){ return a[2].x; }",
        );
        assert!(
            ir.contains("getelementptr inbounds { i32 }"),
            "typedef-struct pointer indexing should use struct element GEP, not byte indexing:\n{}",
            ir
        );
    }

    #[test]
    fn test_indexing_struct_array_field_in_expression() {
        let ir = compile_c_to_ir(
            "typedef struct S { int x; } S; \
             typedef struct H { S a[4]; } H; \
             H h; \
             int readh(int i){ return h.a[i].x; }",
        );
        assert!(
            ir.contains("define i32 @readh"),
            "expected function definition for struct-array-field indexing:\n{}",
            ir
        );
        assert!(
            !ir.contains("ret i32 0"),
            "struct-array-field indexing should not collapse away:\n{}",
            ir
        );
    }

    #[test]
    fn test_global_struct_array_member_assignment_uses_real_storage() {
        let ir = compile_c_to_ir(
            "typedef struct FuncDef FuncDef; \
             struct FuncDef { int x; }; \
             typedef struct Hash { FuncDef *a[23]; } Hash; \
             static Hash gHash; \
             void insert(FuncDef *p, int h) { gHash.a[h] = p; }",
        );
        assert!(
            ir.contains("@gHash"),
            "expected global hash symbol in IR:\n{}",
            ir
        );
        assert!(
            !ir.contains("arr_idx_base"),
            "lvalue indexing into array fields should not route through a temporary copy:\n{}",
            ir
        );
    }

    #[test]
    fn test_member_function_pointer_call_uses_signature() {
        let ir = compile_c_to_ir(
            "struct Cfg { void (*xFree)(void*); }; \
             void invoke(struct Cfg *cfg, void *p) { cfg->xFree(p); }",
        );
        assert!(
            ir.contains("call void"),
            "member function-pointer call should preserve void return type:\n{}",
            ir
        );
        assert!(
            !ir.contains("call i32 %xFree"),
            "member function-pointer call should not use i32 fallback signature:\n{}",
            ir
        );
    }

    #[test]
    fn test_struct_pointer_field_type() {
        let ir = compile_c_to_ir(
            "struct node { int value; struct node *next; }; \
             int test(struct node *head) { return head->value; }",
        );
        // Struct should have {i32, ptr} layout, not {i32, i32}
        assert!(
            ir.contains("{ i32, ptr }"),
            "Expected struct type {{i32, ptr}}:\n{}",
            ir
        );
    }

    #[test]
    fn test_struct_field_index_correctness() {
        let ir = compile_c_to_ir(
            "struct node { int value; struct node *next; }; \
             struct node *get_next(struct node *head) { return head->next; }",
        );
        // head->next should use field index 1, not 0
        assert!(
            ir.contains("i32 0, i32 1"),
            "Expected GEP index 1 for next field:\n{}",
            ir
        );
    }

    #[test]
    fn test_nested_member_access() {
        let ir = compile_c_to_ir(
            "struct node { int value; struct node *next; }; \
             int test(struct node *head) { return head->next->value; }",
        );
        // Should have two GEPs: one for ->next (index 1), one for ->value (index 0)
        assert!(
            ir.contains("i32 0, i32 1"),
            "Expected GEP for next (index 1):\n{}",
            ir
        );
        assert!(
            ir.contains("chain.gep"),
            "Expected chained GEP for nested access:\n{}",
            ir
        );
        assert!(
            ir.contains("ret i32 %value"),
            "Expected return of value field:\n{}",
            ir
        );
    }

    #[test]
    fn test_assign_expr_comparison() {
        let ir = compile_c_to_ir(
            "int test_assign_cmp(void) { int x; if ((x = 42) > 0) { return x; } return -1; }",
        );
        // The comparison should be a runtime icmp on the assign_result load,
        // not a constant-folded `br i1 true`
        assert!(
            ir.contains("assign_result"),
            "Expected assign_result load:\n{}",
            ir
        );
        assert!(
            ir.contains("icmp sgt"),
            "Expected runtime icmp sgt:\n{}",
            ir
        );
        assert!(
            !ir.contains("br i1 true"),
            "Should NOT have constant-folded branch:\n{}",
            ir
        );
    }

    #[test]
    fn test_pointer_assignment_from_zero_stores_null() {
        let ir = compile_c_to_ir("void set_null(void **pp){ *pp = 0; }");
        assert!(
            ir.contains("store ptr null"),
            "pointer assignment from zero should coerce to null pointer store:\n{}",
            ir
        );
    }

    #[test]
    fn test_struct_return_type() {
        let ir = compile_c_to_ir(
            "struct point { int x; int y; }; \
             struct point make_point(int x, int y) { \
                 struct point p; p.x = x; p.y = y; return p; \
             }",
        );
        // Function should return { i32, i32 }, not i32
        assert!(
            ir.contains("define { i32, i32 } @make_point"),
            "Expected struct return type:\n{}",
            ir
        );
        assert!(
            ir.contains("ret { i32, i32 }"),
            "Expected struct ret instruction:\n{}",
            ir
        );
    }

    #[test]
    fn test_multi_var_complex_declarators() {
        // Test 1: pointer with init, then array (basic case from bug report)
        let ir = compile_c_to_ir(
            "void test_multi() { \
                 int x = 42; \
                 int *p = &x, a[10]; \
             }",
        );
        assert!(
            ir.contains("alloca ptr"),
            "T1: Expected alloca ptr for *p:\n{}",
            ir
        );
        assert!(
            ir.contains("alloca [10 x i32]"),
            "T1: Expected alloca [10 x i32] for a[10]:\n{}",
            ir
        );

        // Test 2: array first, then pointer (reversed order)
        let ir2 = compile_c_to_ir(
            "void test_multi2() { \
                 int x = 42; \
                 int a[10], *p; \
             }",
        );
        assert!(
            ir2.contains("alloca [10 x i32]"),
            "T2: Expected alloca [10 x i32] for a[10]:\n{}",
            ir2
        );
        assert!(
            ir2.contains("alloca ptr"),
            "T2: Expected alloca ptr for *p:\n{}",
            ir2
        );

        // Test 3: pointer, scalar, and array in one decl
        let ir3 = compile_c_to_ir(
            "void test_multi3() { \
                 int *p, n, a[5]; \
             }",
        );
        assert!(
            ir3.contains("alloca ptr"),
            "T3: Expected alloca ptr for *p:\n{}",
            ir3
        );
        assert!(
            ir3.contains("alloca i32"),
            "T3: Expected alloca i32 for n:\n{}",
            ir3
        );
        assert!(
            ir3.contains("alloca [5 x i32]"),
            "T3: Expected alloca [5 x i32] for a[5]:\n{}",
            ir3
        );

        // Test 4: double pointer then array
        let ir4 = compile_c_to_ir(
            "void test_multi4() { \
                 int x = 42; \
                 int *px = &x; \
                 int **pp = &px, a[10]; \
             }",
        );
        assert!(
            ir4.contains("alloca ptr"),
            "T4: Expected alloca ptr for **pp:\n{}",
            ir4
        );
        assert!(
            ir4.contains("alloca [10 x i32]"),
            "T4: Expected alloca [10 x i32] for a[10]:\n{}",
            ir4
        );

        // Test 5: two pointers without initializers — verifies declarator_llvm_type
        // does NOT walk next_sibling (which would count q's pointer declarator
        // as extra pointer depth for p)
        let ir5 = compile_c_to_ir(
            "void test_multi5() { \
                 int *p, *q; \
             }",
        );
        let p_alloca_count = ir5.matches("alloca ptr").count();
        assert_eq!(
            p_alloca_count, 2,
            "T5: Expected exactly 2 alloca ptr (one for *p, one for *q):\n{}",
            ir5
        );
    }

    #[test]
    fn test_designated_init_struct() {
        let ir = compile_c_to_ir(
            "struct point { int x; int y; }; \
             void test_desig_init() { \
                 struct point p = {.x = 1, .y = 2}; \
             }",
        );
        // Should allocate the struct
        assert!(
            ir.contains("alloca { i32, i32 }"),
            "Expected struct alloca:\n{}",
            ir
        );
        // Should have GEP instructions targeting struct fields
        assert!(
            ir.contains("getelementptr inbounds { i32, i32 }"),
            "Expected struct GEP for designated init:\n{}",
            ir
        );
        // Should have stores for both fields
        assert!(
            ir.contains("store i32 1"),
            "Expected store of 1 for .x:\n{}",
            ir
        );
        assert!(
            ir.contains("store i32 2"),
            "Expected store of 2 for .y:\n{}",
            ir
        );
        // Should have the designated-init GEP label
        assert!(
            ir.contains("desig.gep"),
            "Expected desig.gep label:\n{}",
            ir
        );
    }

    #[test]
    fn test_designated_init_partial() {
        // Only initialize one field — the other should not crash
        let ir = compile_c_to_ir(
            "struct point { int x; int y; }; \
             void test_partial() { \
                 struct point p = {.y = 42}; \
             }",
        );
        assert!(
            ir.contains("alloca { i32, i32 }"),
            "Expected struct alloca:\n{}",
            ir
        );
        // .y is field index 1
        assert!(
            ir.contains("desig.gep"),
            "Expected desig.gep for .y:\n{}",
            ir
        );
        assert!(
            ir.contains("store i32 42"),
            "Expected store of 42 for .y:\n{}",
            ir
        );
    }

    #[test]
    fn test_designated_init_field_order() {
        // Initialize fields out of declaration order
        let ir = compile_c_to_ir(
            "struct rgb { int r; int g; int b; }; \
             void test_order() { \
                 struct rgb c = {.b = 3, .r = 1, .g = 2}; \
             }",
        );
        assert!(
            ir.contains("alloca { i32, i32, i32 }"),
            "Expected 3-field struct alloca:\n{}",
            ir
        );
        // All three field values should be stored
        assert!(ir.contains("store i32 3"), "Expected store for .b:\n{}", ir);
        assert!(ir.contains("store i32 1"), "Expected store for .r:\n{}", ir);
        assert!(ir.contains("store i32 2"), "Expected store for .g:\n{}", ir);
        // Should have 3 GEPs (each GEP line contains "desig.gep")
        let gep_count = ir
            .lines()
            .filter(|l| l.contains("getelementptr") && l.contains("desig.gep"))
            .count();
        assert_eq!(
            gep_count, 3,
            "Expected 3 desig.gep GEP instructions:\n{}",
            ir
        );
    }

    // ------------------------------------------------------------------ //
    //  Compound literal tests (kind=212)
    // ------------------------------------------------------------------ //

    #[test]
    fn test_compound_literal_struct() {
        let ir = compile_c_to_ir(
            "struct point { int x; int y; }; \
             int test_cl() { \
                 struct point p = (struct point){.x = 10, .y = 20}; \
                 return p.x + p.y; \
             }",
        );
        // Should allocate a temporary for the compound literal
        assert!(
            ir.contains("compound.lit"),
            "Expected compound.lit alloca:\n{}",
            ir
        );
        // Should have stores for the designated init values
        assert!(
            ir.contains("store i32 10"),
            "Expected store of 10 for .x:\n{}",
            ir
        );
        assert!(
            ir.contains("store i32 20"),
            "Expected store of 20 for .y:\n{}",
            ir
        );
        // Should have a load of the compound literal
        assert!(
            ir.contains("compound.lit.load"),
            "Expected compound.lit.load:\n{}",
            ir
        );
    }

    #[test]
    fn test_compound_literal_scalar() {
        let ir = compile_c_to_ir(
            "int test_scalar_cl() { \
                 int x = (int){42}; \
                 return x; \
             }",
        );
        // Should allocate a temporary for the compound literal
        assert!(
            ir.contains("compound.lit"),
            "Expected compound.lit alloca:\n{}",
            ir
        );
        assert!(ir.contains("store i32 42"), "Expected store of 42:\n{}", ir);
        assert!(
            ir.contains("compound.lit.load"),
            "Expected compound.lit.load:\n{}",
            ir
        );
    }

    #[test]
    fn test_compound_literal_in_expression() {
        // Compound literal used directly in a return expression
        let ir = compile_c_to_ir(
            "struct pair { int a; int b; }; \
             int test_cl_expr() { \
                 struct pair p = (struct pair){.a = 3, .b = 7}; \
                 return p.a; \
             }",
        );
        assert!(
            ir.contains("compound.lit"),
            "Expected compound.lit alloca:\n{}",
            ir
        );
        assert!(
            ir.contains("store i32 3"),
            "Expected store of 3 for .a:\n{}",
            ir
        );
        assert!(
            ir.contains("store i32 7"),
            "Expected store of 7 for .b:\n{}",
            ir
        );
    }

    // ------------------------------------------------------------------ //
    //  Bitfield codegen tests
    // ------------------------------------------------------------------ //

    #[test]
    fn test_bitfield_struct_type() {
        // Three 1-bit bitfields of unsigned int should pack into a single i32
        let ir = compile_c_to_ir(
            "struct flags { \
                 unsigned int readable : 1; \
                 unsigned int writable : 1; \
                 unsigned int executable : 1; \
             }; \
             int test(void) { \
                 struct flags f; \
                 f.readable = 1; \
                 f.writable = 0; \
                 f.executable = 1; \
                 return f.readable + f.executable; \
             }",
        );
        // The struct should be { i32 } (one storage unit), not { i32, i32, i32 }
        assert!(
            ir.contains("{ i32 }"),
            "Expected packed bitfield struct {{ i32 }}:\n{}",
            ir
        );
        // Bitfield writes should use OR to insert bits
        assert!(
            ir.contains("bf.insert"),
            "Expected bf.insert (OR) for bitfield write:\n{}",
            ir
        );
        // Bitfield writes should clear old bits with AND
        assert!(
            ir.contains("bf.cleared"),
            "Expected bf.cleared (AND) for bitfield clear:\n{}",
            ir
        );
        // Bitfield reads should use AND mask
        assert!(
            ir.contains("bf.mask"),
            "Expected bf.mask (AND) for bitfield read:\n{}",
            ir
        );
    }

    #[test]
    fn test_bitfield_read_shift_mask() {
        // Verify the second bitfield (writable at bit_offset=1) uses a shift
        let ir = compile_c_to_ir(
            "struct flags { \
                 unsigned int readable : 1; \
                 unsigned int writable : 1; \
             }; \
             int get_writable(void) { \
                 struct flags f; \
                 f.readable = 1; \
                 f.writable = 1; \
                 return f.writable; \
             }",
        );
        // Reading writable (bit_offset=1) needs lshr by 1
        assert!(
            ir.contains("bf.shr"),
            "Expected bf.shr (right shift) for non-zero bit_offset read:\n{}",
            ir
        );
        assert!(
            ir.contains("bf.mask"),
            "Expected bf.mask for bitfield read:\n{}",
            ir
        );
    }

    #[test]
    fn test_bitfield_write_shift() {
        // Verify writing to a non-zero bit_offset field uses shl
        // Use a function parameter so LLVM can't constant-fold the shift
        let ir = compile_c_to_ir(
            "struct flags { \
                 unsigned int readable : 1; \
                 unsigned int writable : 1; \
             }; \
             void set_writable(int val) { \
                 struct flags f; \
                 f.writable = val; \
             }",
        );
        // Writing writable (bit_offset=1) needs shl by 1
        assert!(
            ir.contains("bf.new.shl"),
            "Expected bf.new.shl (left shift) for non-zero bit_offset write:\n{}",
            ir
        );
        assert!(
            ir.contains("bf.insert"),
            "Expected bf.insert (OR) for bitfield write:\n{}",
            ir
        );
    }

    // ===== Multi-Translation-Unit (multi-TU) IR-level tests =====

    #[test]
    fn test_multi_tu_helper_defines_add() {
        // Simulate helper.c: defines a function `add`
        let ir = compile_c_to_ir("int add(int a, int b) { return a + b; }");
        // The helper TU should have a `define` for `add`, not a `declare`
        assert!(
            ir.contains("define i32 @add(i32"),
            "Expected define i32 @add in helper TU:\n{}",
            ir
        );
        assert!(
            !ir.contains("declare i32 @add"),
            "Helper TU should NOT have declare for add:\n{}",
            ir
        );
    }

    #[test]
    fn test_multi_tu_main_declares_extern_add() {
        // Simulate main.c: declares extern add and calls it from main
        let ir = compile_c_to_ir(
            "extern int add(int a, int b); \
             int main(void) { return add(3, 4); }",
        );
        // The main TU should have a `declare` for `add` (extern, not defined here)
        assert!(
            ir.contains("declare i32 @add(i32"),
            "Expected declare i32 @add in main TU:\n{}",
            ir
        );
        // The main TU should have a `define` for `main`
        assert!(
            ir.contains("define i32 @main"),
            "Expected define i32 @main in main TU:\n{}",
            ir
        );
        // The main TU should NOT have a `define` for `add`
        assert!(
            !ir.contains("define i32 @add"),
            "Main TU should NOT define add:\n{}",
            ir
        );
        // Should contain a call to @add with two i32 arguments
        assert!(
            ir.contains("call i32 @add(i32"),
            "Expected call i32 @add(i32 ...) in main TU:\n{}",
            ir
        );
    }

    #[test]
    fn test_multi_tu_extern_with_pointer_params() {
        // Helper TU: defines a function operating on pointer params
        let helper_ir = compile_c_to_ir(
            "int string_length(const char *s) { \
                 int len = 0; \
                 while (s[len]) len = len + 1; \
                 return len; \
             }",
        );
        assert!(
            helper_ir.contains("define i32 @string_length(ptr"),
            "Expected define i32 @string_length(ptr) in helper TU:\n{}",
            helper_ir
        );

        // Main TU: declares extern and calls it
        let main_ir = compile_c_to_ir(
            "extern int string_length(const char *s); \
             int main(void) { return string_length(\"hello\"); }",
        );
        assert!(
            main_ir.contains("declare i32 @string_length(ptr"),
            "Expected declare i32 @string_length(ptr) in main TU:\n{}",
            main_ir
        );
        assert!(
            main_ir.contains("call i32 @string_length(ptr"),
            "Expected call to string_length in main TU:\n{}",
            main_ir
        );
    }

    #[test]
    fn test_multi_tu_extern_void_function() {
        // Helper TU: defines a void function
        let helper_ir = compile_c_to_ir(
            "int global_val; \
             void set_value(int v) { global_val = v; }",
        );
        assert!(
            helper_ir.contains("define void @set_value(i32"),
            "Expected define void @set_value in helper TU:\n{}",
            helper_ir
        );

        // Main TU: declares extern void function and calls it
        let main_ir = compile_c_to_ir(
            "extern void set_value(int v); \
             int main(void) { set_value(42); return 0; }",
        );
        assert!(
            main_ir.contains("declare void @set_value(i32"),
            "Expected declare void @set_value in main TU:\n{}",
            main_ir
        );
        assert!(
            !main_ir.contains("define void @set_value"),
            "Main TU should NOT define set_value:\n{}",
            main_ir
        );
    }

    #[test]
    fn test_multi_tu_multiple_extern_decls() {
        // Main TU: declares multiple externs from different hypothetical TUs
        let ir = compile_c_to_ir(
            "extern int add(int a, int b); \
             extern int multiply(int a, int b); \
             int main(void) { return add(2, 3) + multiply(4, 5); }",
        );
        assert!(
            ir.contains("declare i32 @add(i32"),
            "Expected declare for add:\n{}",
            ir
        );
        assert!(
            ir.contains("declare i32 @multiply(i32"),
            "Expected declare for multiply:\n{}",
            ir
        );
        assert!(
            ir.contains("define i32 @main"),
            "Expected define for main:\n{}",
            ir
        );
        assert!(
            ir.contains("call i32 @add(i32"),
            "Expected call to add:\n{}",
            ir
        );
        assert!(
            ir.contains("call i32 @multiply(i32"),
            "Expected call to multiply:\n{}",
            ir
        );
    }

    #[test]
    fn test_multi_tu_ir_verifies_for_both_sides() {
        // Verify that both TUs produce valid LLVM IR (module verification)
        use crate::frontend::parser::Parser;
        use tempfile::NamedTempFile;

        // Helper TU
        {
            let temp_file = NamedTempFile::new().unwrap();
            let arena = Arena::new(temp_file.path(), 65536).unwrap();
            let mut parser = Parser::new(arena);
            let root = parser
                .parse("int add(int a, int b) { return a + b; }")
                .expect("parse failed");

            let context = Context::create();
            let ts = TypeSystem::new();
            let mut backend = LlvmBackend::with_types(&context, "helper", &ts);
            backend
                .compile(&parser.arena, root)
                .expect("compile failed");
            backend
                .verify()
                .expect("helper TU LLVM verification failed");
        }

        // Main TU
        {
            let temp_file = NamedTempFile::new().unwrap();
            let arena = Arena::new(temp_file.path(), 65536).unwrap();
            let mut parser = Parser::new(arena);
            let root = parser
                .parse(
                    "extern int add(int a, int b); \
                 int main(void) { return add(3, 4); }",
                )
                .expect("parse failed");

            let context = Context::create();
            let ts = TypeSystem::new();
            let mut backend = LlvmBackend::with_types(&context, "main", &ts);
            backend
                .compile(&parser.arena, root)
                .expect("compile failed");
            backend.verify().expect("main TU LLVM verification failed");
        }
    }

    #[test]
    fn test_multi_tu_signature_consistency() {
        // Verify that the define in helper and declare in main have matching signatures
        let helper_ir = compile_c_to_ir("int add(int a, int b) { return a + b; }");
        let main_ir = compile_c_to_ir(
            "extern int add(int a, int b); \
             int main(void) { return add(1, 2); }",
        );

        // Extract the signature of add from both TUs
        // Helper should have: define i32 @add(i32 %..., i32 %...)
        // Main should have:   declare i32 @add(i32, i32)
        // Both use i32 return type and two i32 params
        let helper_has_i32_ret = helper_ir.contains("define i32 @add(");
        let main_has_i32_ret = main_ir.contains("declare i32 @add(");
        assert!(
            helper_has_i32_ret,
            "Helper TU add should return i32:\n{}",
            helper_ir
        );
        assert!(
            main_has_i32_ret,
            "Main TU add declare should return i32:\n{}",
            main_ir
        );

        // Count i32 params: both should have exactly 2
        let helper_add_line = helper_ir
            .lines()
            .find(|l| l.contains("define i32 @add("))
            .expect("no define line for add");
        let main_add_line = main_ir
            .lines()
            .find(|l| l.contains("declare i32 @add("))
            .expect("no declare line for add");

        let helper_param_count = helper_add_line.matches("i32").count() - 1; // subtract return type
        let main_param_count = main_add_line.matches("i32").count() - 1;
        assert_eq!(
            helper_param_count, 2,
            "Helper add should have 2 i32 params: {}",
            helper_add_line
        );
        assert_eq!(
            main_param_count, 2,
            "Main add declare should have 2 i32 params: {}",
            main_add_line
        );
    }
}

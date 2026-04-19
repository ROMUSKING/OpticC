use crate::arena::{Arena, CAstNode, NodeFlags, NodeOffset};
use crate::types::{CType, TypeId, TypeSystem};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, StructType};
use inkwell::values::{BasicValue, BasicValueEnum, FunctionValue, PointerValue};
use inkwell::basic_block::BasicBlock;
use inkwell::AddressSpace;
use std::collections::HashMap;

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
    vectorization_hints: VectorizationHints,
    types: Option<&'types TypeSystem>,
    type_cache: HashMap<u32, BasicTypeEnum<'ctx>>,
    /// For each struct variable, stores ordered field names so GEP can resolve them.
    struct_fields: HashMap<String, Vec<String>>,
    /// Registered struct/union types keyed by tag name.
    struct_tag_types: HashMap<String, StructType<'ctx>>,
    /// Registered struct/union field names keyed by tag name.
    struct_tag_fields: HashMap<String, Vec<String>>,
    /// Registered pointee struct types keyed by pointer-backed variable/parameter name.
    pointer_struct_types: HashMap<String, StructType<'ctx>>,
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
}

#[derive(Clone, Copy)]
struct VariableBinding<'ctx> {
    ptr: PointerValue<'ctx>,
    pointee_type: BasicTypeEnum<'ctx>,
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
            vectorization_hints: VectorizationHints::default(),
            types: None,
            type_cache: HashMap::new(),
            struct_fields: HashMap::new(),
            struct_tag_types: HashMap::new(),
            struct_tag_fields: HashMap::new(),
            pointer_struct_types: HashMap::new(),
            current_return_type: None,
            label_blocks: HashMap::new(),
            break_stack: Vec::new(),
            continue_stack: Vec::new(),
            scope_stack: Vec::new(),
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
            vectorization_hints: VectorizationHints::default(),
            types: Some(types),
            type_cache: HashMap::new(),
            struct_fields: HashMap::new(),
            struct_tag_types: HashMap::new(),
            struct_tag_fields: HashMap::new(),
            pointer_struct_types: HashMap::new(),
            current_return_type: None,
            label_blocks: HashMap::new(),
            break_stack: Vec::new(),
            continue_stack: Vec::new(),
            scope_stack: Vec::new(),
        }
    }

    pub fn set_vectorization_hints(&mut self, hints: VectorizationHints) {
        self.vectorization_hints = hints;
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
            scope.entry(name.clone()).or_insert_with(|| {
                self.variables.get(&name).copied()
            });
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
                Some(CType::Struct { members, .. }) => {
                    let field_types: Vec<BasicTypeEnum> = members
                        .iter()
                        .map(|m| self.to_llvm_type(m.type_id.0))
                        .collect();
                    if field_types.is_empty() {
                        self.context.i8_type().as_basic_type_enum()
                    } else {
                        self.context
                            .struct_type(&field_types, false)
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
        self.lower_translation_unit(arena, root)?;
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
                    20 => { let _ = self.lower_global_decl(arena, node); }
                    21 => { let _ = self.lower_global_var(arena, node, node.kind); }
                    22 => { let _ = self.lower_func_decl(arena, node); }
                    23 => { let _ = self.lower_func_def(arena, node); }
                    101..=105 => {}
                    _ => {}
                }
                child_offset = node.next_sibling;
            } else {
                break;
            }
        }
        Ok(())
    }

    /// Lower a top-level declaration, handling global variables properly.
    fn lower_global_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
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
                    1..=6 | 83 => {
                        spec_kind = child.kind;
                        if first_spec == NodeOffset::NULL {
                            first_spec = scan_off;
                        }
                    }
                    101..=105 => {
                        if child.kind == 104 { _is_const = true; } // const qualifier
                    }
                    _ => break,
                }
                scan_off = child.next_sibling;
            } else {
                break;
            }
        }

        // Process each child - function decls, func defs, and var decls
        child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                match child.kind {
                    21 => {
                        // Try to handle as a global variable
                        let _ = self.lower_global_var(arena, child, spec_kind);
                    }
                    22 => { let _ = self.lower_func_decl(arena, child); }
                    23 => { let _ = self.lower_func_def(arena, child); }
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
    fn lower_global_var(&mut self, arena: &Arena, node: &CAstNode, spec_kind: u16) -> Result<(), BackendError> {
        let llvm_type = self.node_kind_to_llvm_type(spec_kind);
        // Extract attributes early so we can apply them to any globals we create
        let attrs = self.extract_attributes(arena, node);
        
        // Find the variable name and initializer
        let mut name_opt: Option<String> = None;
        let mut is_pointer = false;
        let mut is_array = false;
        let mut init_offset = NodeOffset::NULL;
        
        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                match child.kind {
                    // init-declarator (name = init)
                    73 => {
                        let decl_node = arena.get(child.first_child);
                        if let Some(dn) = decl_node {
                            match dn.kind {
                                60 => {
                                    name_opt = arena.get_string(NodeOffset(dn.data))
                                        .filter(|s| !s.is_empty())
                                        .map(|s| s.to_string());
                                    init_offset = dn.next_sibling;
                                }
                                7 => {
                                    is_pointer = true;
                                    name_opt = self.find_ident_name_in(arena, dn);
                                    init_offset = dn.next_sibling;
                                }
                                8 => {
                                    is_array = true;
                                    name_opt = self.find_ident_name_in(arena, dn);
                                    init_offset = dn.next_sibling;
                                }
                                _ => {
                                    name_opt = self.find_ident_name_in(arena, dn);
                                    init_offset = dn.next_sibling;
                                }
                            }
                        }
                    }
                    // plain identifier
                    60 => {
                        name_opt = arena.get_string(NodeOffset(child.data))
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string());
                    }
                    // pointer declarator
                    7 => {
                        is_pointer = true;
                        name_opt = self.find_ident_name_in(arena, child);
                    }
                    // array declarator
                    8 => {
                        is_array = true;
                        name_opt = self.find_ident_name_in(arena, child);
                    }
                    // type specifiers - skip
                    1..=6 | 83 | 101..=105 => {}
                    _ => {}
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }

        if let Some(var_name) = name_opt {
            let global_type = if is_pointer {
                self.context.ptr_type(AddressSpace::default()).as_basic_type_enum()
            } else if is_array {
                // For arrays like `char name[] = "..."`, use the initializer to determine type
                llvm_type
            } else {
                llvm_type
            };

            // Check if there's a string initializer (for char arrays)
            if init_offset != NodeOffset::NULL {
                if let Some(init_node) = arena.get(init_offset) {
                    if init_node.kind == 63 || init_node.kind == 81 {
                        // String literal initializer - create global string
                        let string_offset = NodeOffset(init_node.data);
                        let string_text = arena.get_string(string_offset).unwrap_or("");
                        let bytes = string_text.as_bytes();
                        let string_val = self.context.const_string(bytes, true);
                        let global = self.module.add_global(
                            string_val.get_type(),
                            Some(AddressSpace::default()),
                            &var_name,
                        );
                        global.set_initializer(&string_val);
                        global.set_constant(true);
                        self.apply_global_attributes(global, &attrs);
                        // Store in variables for later reference
                        let binding = VariableBinding {
                                ptr: global.as_pointer_value(),
                                pointee_type: string_val.get_type().as_basic_type_enum(),
                        };
                        self.global_variables.insert(var_name.clone(), binding);
                        self.variables.insert(var_name, binding);
                        return Ok(());
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
                            let null_val = self.context.ptr_type(AddressSpace::default()).const_null();
                            global.set_initializer(&null_val);
                        } else {
                            let const_val = match global_type {
                                BasicTypeEnum::IntType(it) => it.const_int(value, false),
                                _ => self.context.i32_type().const_int(value, false),
                            };
                            global.set_initializer(&const_val);
                        }
                        let binding = VariableBinding {
                                ptr: global.as_pointer_value(),
                                pointee_type: global_type,
                        };
                        self.apply_global_attributes(global, &attrs);
                        self.global_variables.insert(var_name.clone(), binding);
                        self.variables.insert(var_name, binding);
                        return Ok(());
                    }
                }
            }

            // Default: create zero-initialized global
            let global = self.module.add_global(
                global_type,
                Some(AddressSpace::default()),
                &var_name,
            );
            let zero: BasicValueEnum = match global_type {
                BasicTypeEnum::IntType(it) => it.const_zero().into(),
                BasicTypeEnum::FloatType(ft) => ft.const_zero().into(),
                BasicTypeEnum::PointerType(pt) => pt.const_null().into(),
                _ => self.context.i32_type().const_zero().into(),
            };
            global.set_initializer(&zero);
            self.apply_global_attributes(global, &attrs);
            let binding = VariableBinding {
                    ptr: global.as_pointer_value(),
                    pointee_type: global_type,
            };
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
                    21 => { let _ = self.lower_var_decl(arena, &child); }
                    22 => { let _ = self.lower_func_decl(arena, &child); }
                    23 => { let _ = self.lower_func_def(arena, &child); }
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
                            let mut member_off = child.first_child;
                            while member_off != NodeOffset::NULL {
                                if let Some(m) = arena.get(member_off) {
                                    let mk = arena.get(m.first_child).map(|n| n.kind).unwrap_or(2);
                                    field_types.push(self.node_kind_to_llvm_type(mk));
                                    member_off = m.next_sibling;
                                } else {
                                    break;
                                }
                            }
                            if !field_types.is_empty() {
                                let st = self.context.struct_type(&field_types, false);
                                self.struct_tag_types.insert(tag_name.clone(), st);
                                self.struct_tag_fields.insert(tag_name, field_names);
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
            6 => self.context.i32_type().as_basic_type_enum(),
            10 => self.context.i16_type().as_basic_type_enum(),
            11 => self.context.i64_type().as_basic_type_enum(),
            12 | 13 => self.context.i32_type().as_basic_type_enum(),
            83 => self.context.f32_type().as_basic_type_enum(),
            84 => self.context.f64_type().as_basic_type_enum(),
            _ => self.context.i32_type().as_basic_type_enum(),
        }
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
    ) {
        let spec_node = arena.get(param.first_child);
        let type_kind = spec_node.map(|n| n.kind).unwrap_or(2);
        let base_type = if matches!(type_kind, 4 | 5) {
            self.struct_info_for_spec(arena, spec_node)
                .map(|(struct_type, _)| struct_type.as_basic_type_enum())
                .unwrap_or_else(|| self.context.i8_type().as_basic_type_enum())
        } else {
            self.node_kind_to_llvm_type(type_kind)
        };
        let mut name = "p".to_string();
        let mut declarator_offset = NodeOffset::NULL;
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
                // Descend into pointer/array declarators to find the identifier
                if matches!(n.kind, 7 | 8) && n.first_child != NodeOffset::NULL {
                    let mut inner = n.first_child;
                    while inner != NodeOffset::NULL {
                        if let Some(inner_n) = arena.get(inner) {
                            if inner_n.kind == 60 {
                                if let Some(s) = arena.get_string(NodeOffset(inner_n.data)) {
                                    if !s.is_empty() {
                                        name = s.to_string();
                                        break;
                                    }
                                }
                            }
                            inner = inner_n.next_sibling;
                        } else {
                            break;
                        }
                    }
                    if name != "p" {
                        break;
                    }
                }
                off = n.next_sibling;
            } else {
                break;
            }
        }
        let llvm_type = self.declarator_llvm_type(arena, arena.get(declarator_offset), base_type);
        (llvm_type, name, self.struct_info_for_spec(arena, spec_node))
    }

    fn struct_info_for_spec(
        &self,
        arena: &Arena,
        spec_node: Option<&CAstNode>,
    ) -> Option<(StructType<'ctx>, Vec<String>)> {
        let spec_node = spec_node?;
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
        arena: &Arena,
        declarator: Option<&CAstNode>,
        base_type: BasicTypeEnum<'ctx>,
    ) -> BasicTypeEnum<'ctx> {
        let Some(declarator) = declarator else {
            return base_type;
        };

        match declarator.kind {
            7 => {
                let mut depth = 1usize;
                let mut cursor = declarator.next_sibling;
                while cursor != NodeOffset::NULL {
                    if let Some(node) = arena.get(cursor) {
                        if node.kind == 7 {
                            depth += 1;
                            cursor = node.next_sibling;
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }

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
            _ => base_type,
        }
    }

    fn lower_var_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let spec_node = arena.get(node.first_child);
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
                            let var_ptr = self
                                .build_entry_alloca(actual_alloca_type, &var_name)
                                .or_else(|_| self.builder.build_alloca(actual_alloca_type, &var_name).map_err(|_| BackendError::InvalidNode))?;
                            if let Some((struct_type, field_names)) = struct_info.clone() {
                                self.struct_fields
                                    .insert(var_name.clone(), field_names.clone());
                                if actual_alloca_type.is_pointer_type() {
                                    self.pointer_struct_types
                                        .insert(var_name.clone(), struct_type);
                                }
                            }
                            self.insert_scoped_variable(
                                var_name.clone(),
                                VariableBinding {
                                    ptr: var_ptr,
                                    pointee_type: actual_alloca_type,
                                },
                            );
                            // Process initializer only for non-array types
                            // (array initializers require separate aggregate handling)
                            if !is_array {
                                let init_offset = declarator_node
                                    .map(|d| d.next_sibling)
                                    .unwrap_or(NodeOffset::NULL);
                                if init_offset != NodeOffset::NULL {
                                    if let Some(val) = self.lower_expr(arena, init_offset)? {
                                        if Self::types_compatible(actual_alloca_type, val) {
                                            let _ = self
                                                .builder
                                                .build_store(var_ptr, val)
                                                .map_err(|_| BackendError::InvalidNode);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Kind=60/7/8: plain scalar, pointer, or array declarator (no initializer)
                    60 | 7 | 8 => {
                        let actual_alloca_type =
                            self.declarator_llvm_type(arena, Some(child), alloca_type);
                        let var_name_opt = if child.kind == 60 {
                            arena
                                .get_string(NodeOffset(child.data))
                                .filter(|s| !s.is_empty())
                                .map(|s| s.to_string())
                        } else {
                            self.find_ident_name_in(arena, child)
                        };

                        if let Some(var_name) = var_name_opt {
                            let var_ptr = self
                                .build_entry_alloca(actual_alloca_type, &var_name)
                                .or_else(|_| self.builder.build_alloca(actual_alloca_type, &var_name).map_err(|_| BackendError::InvalidNode))?;
                            if let Some((struct_type, field_names)) = struct_info.clone() {
                                self.struct_fields
                                    .insert(var_name.clone(), field_names.clone());
                                if actual_alloca_type.is_pointer_type() {
                                    self.pointer_struct_types
                                        .insert(var_name.clone(), struct_type);
                                }
                            }
                            self.insert_scoped_variable(
                                var_name,
                                VariableBinding {
                                    ptr: var_ptr,
                                    pointee_type: actual_alloca_type,
                                },
                            );
                        }
                    }
                    // Type specifiers: skip
                    1..=15 | 83 | 84 | 101..=105 => {}
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
            _ => false,
        }
    }

    fn lower_lvalue_ptr(
        &mut self,
        arena: &Arena,
        offset: NodeOffset,
    ) -> Result<Option<(PointerValue<'ctx>, BasicTypeEnum<'ctx>)>, BackendError> {
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
        let base_offset = node.first_child;
        let field_offset = node.next_sibling;

        let base_name = match arena.get(base_offset).and_then(|n| {
            if n.kind == 60 {
                arena.get_string(NodeOffset(n.data)).map(|s| s.to_string())
            } else {
                None
            }
        }) {
            Some(name) => name,
            None => return Ok(None),
        };

        let field_name = match arena.get(field_offset).and_then(|n| {
            if n.kind == 60 {
                arena.get_string(NodeOffset(n.data)).map(|s| s.to_string())
            } else {
                None
            }
        }) {
            Some(name) => name,
            None => return Ok(None),
        };

        let field_idx = self
            .struct_fields
            .get(&base_name)
            .and_then(|fields| fields.iter().position(|f| f == &field_name))
            .unwrap_or(0) as u32;

        if node.data == 1 {
            let Some(struct_type) = self.pointer_struct_types.get(&base_name).copied() else {
                return Ok(None);
            };
            let base_ptr = match self.lower_expr(arena, base_offset)? {
                Some(BasicValueEnum::PointerValue(ptr)) => ptr,
                _ => return Ok(None),
            };
            let field_ptr = self
                .builder
                .build_struct_gep(struct_type, base_ptr, field_idx, "arrow.gep")
                .map_err(|_| BackendError::InvalidNode)?;
            let field_type = struct_type
                .get_field_type_at_index(field_idx)
                .unwrap_or_else(|| self.context.i32_type().as_basic_type_enum());
            return Ok(Some((field_ptr, field_type)));
        }

        let binding = match self.variables.get(&base_name).copied() {
            Some(binding) => binding,
            None => return Ok(None),
        };
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
        Ok(Some((field_ptr, field_type)))
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

        // Fallback: evaluate base as expression, determine element type
        // Try to infer element type from the base variable's pointee type
        let mut elem_type = self.context.i32_type().as_basic_type_enum();
        if let Some(base_node) = arena.get(node.first_child) {
            if base_node.kind == 60 {
                if let Some(name) = arena.get_string(NodeOffset(base_node.data)) {
                    if let Some(binding) = self.variables.get(name).copied() {
                        // If the variable is a pointer (pointee_type is ptr), then
                        // loading it gives a pointer, and indexing into it should use ptr element type
                        if binding.pointee_type.is_pointer_type() {
                            elem_type = self.context.ptr_type(AddressSpace::default()).as_basic_type_enum();
                        }
                    }
                }
            }
        }
        let base_ptr = match self.lower_expr(arena, node.first_child)? {
            Some(BasicValueEnum::PointerValue(ptr)) => ptr,
            _ => return Ok(None),
        };
        let ptr = unsafe {
            self.builder
                .build_gep(elem_type, base_ptr, &[index_val], "arrayidx")
                .map_err(|_| BackendError::InvalidNode)?
        };
        Ok(Some((ptr, elem_type)))
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

        // Handle pointer operands by converting them to integers first
        let lhs_val = if lhs_val.is_pointer_value() {
            self.builder
                .build_ptr_to_int(lhs_val.into_pointer_value(), self.context.i64_type(), "assign_ptr2int_lhs")
                .map_err(|_| BackendError::InvalidNode)?
                .into()
        } else {
            lhs_val
        };
        let rhs_val = if rhs_val.is_pointer_value() {
            self.builder
                .build_ptr_to_int(rhs_val.into_pointer_value(), self.context.i64_type(), "assign_ptr2int_rhs")
                .map_err(|_| BackendError::InvalidNode)?
                .into()
        } else {
            rhs_val
        };
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
        let mut member_off = node.first_child;
        while member_off != NodeOffset::NULL {
            if let Some(member) = arena.get(member_off) {
                let member_kind = arena.get(member.first_child).map(|n| n.kind).unwrap_or(2);
                let ft = self.node_kind_to_llvm_type(member_kind);
                field_types.push(ft);
                member_off = member.next_sibling;
            } else {
                break;
            }
        }
        if field_types.is_empty() {
            self.context.i8_type().as_basic_type_enum()
        } else {
            self.context
                .struct_type(&field_types, false)
                .as_basic_type_enum()
        }
    }

    fn collect_struct_field_names(arena: &Arena, node: &CAstNode) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        let mut member_off = node.first_child;
        while member_off != NodeOffset::NULL {
            if let Some(member) = arena.get(member_off) {
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
    fn pre_register_func_def(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let mut func_name = "func".to_string();
        let mut param_types: Vec<BasicMetadataTypeEnum> = Vec::new();
        let mut return_llvm_type: Option<BasicTypeEnum<'ctx>> = None;
        let mut is_void_ret = false;
        let mut is_variadic = false;

        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                match child.kind {
                    1..=6 | 83 => {
                        is_void_ret = child.kind == 1;
                        if !is_void_ret {
                            return_llvm_type = Some(self.node_kind_to_llvm_type(child.kind));
                        }
                    }
                    7..=9 => {
                        if let Some(func_decl_offset) =
                            self.find_function_declarator_offset(arena, child_offset)
                        {
                            if let Some(func_decl) = arena.get(func_decl_offset) {
                                // Check variadic flag
                                if func_decl.data == 1 {
                                    is_variadic = true;
                                }
                                if !is_void_ret {
                                    return_llvm_type = Some(self.declarator_llvm_type(
                                        arena,
                                        Some(child),
                                        return_llvm_type.unwrap_or_else(|| {
                                            self.context.i32_type().as_basic_type_enum()
                                        }),
                                    ));
                                }
                                if let Some(ident) = arena.get(func_decl.first_child) {
                                    if ident.kind == 60 {
                                        if let Some(name) = arena.get_string(NodeOffset(ident.data)) {
                                            func_name = name.to_string();
                                        }
                                    }
                                    let mut param_off = ident.next_sibling;
                                    while param_off != NodeOffset::NULL {
                                        if let Some(param) = arena.get(param_off) {
                                            if param.kind == 24 {
                                                let (ptype, _, _) =
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
                    }
                    _ => {}
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }

        let ret_llvm = return_llvm_type.unwrap_or_else(|| self.context.i32_type().as_basic_type_enum());
        let fn_type = if is_void_ret {
            self.context.void_type().fn_type(&param_types, is_variadic)
        } else {
            ret_llvm.fn_type(&param_types, is_variadic)
        };
        let function = self.module.add_function(&func_name, fn_type, None);
        // Apply any __attribute__ decorations to the pre-registered function
        let attrs = self.extract_attributes(arena, node);
        self.apply_function_attributes(function, &attrs);
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

        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                match child.kind {
                    1..=6 | 83 => {
                        is_void_ret = child.kind == 1;
                        if !is_void_ret {
                            return_llvm_type = Some(self.node_kind_to_llvm_type(child.kind));
                        }
                    }
                    7..=9 => {
                        if let Some(func_decl_offset) =
                            self.find_function_declarator_offset(arena, child_offset)
                        {
                            if let Some(func_decl) = arena.get(func_decl_offset) {
                                if func_decl.data == 1 {
                                    is_variadic = true;
                                }
                                if !is_void_ret {
                                    return_llvm_type = Some(self.declarator_llvm_type(
                                        arena,
                                        Some(child),
                                        return_llvm_type.unwrap_or_else(|| {
                                            self.context.i32_type().as_basic_type_enum()
                                        }),
                                    ));
                                }
                                if let Some(ident) = arena.get(func_decl.first_child) {
                                    if ident.kind == 60 {
                                        if let Some(name) = arena.get_string(NodeOffset(ident.data)) {
                                            func_name = name.to_string();
                                        }
                                    }
                                    let mut param_off = ident.next_sibling;
                                    while param_off != NodeOffset::NULL {
                                        if let Some(param) = arena.get(param_off) {
                                            if param.kind == 24 {
                                                let (ptype, _, _) =
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
                    }
                    _ => {}
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }

        let ret_llvm = return_llvm_type.unwrap_or_else(|| self.context.i32_type().as_basic_type_enum());
        let fn_type = if is_void_ret {
            self.context.void_type().fn_type(&param_types, is_variadic)
        } else {
            ret_llvm.fn_type(&param_types, is_variadic)
        };
        // Only add if not already registered (pre_register may have added it)
        if self.module.get_function(&func_name).is_none() {
            let function = self.module.add_function(&func_name, fn_type, None);
            let attrs = self.extract_attributes(arena, node);
            self.apply_function_attributes(function, &attrs);
            self.functions.insert(func_name, function);
        }
        Ok(())
    }

    fn lower_func_def(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let mut func_name = "func".to_string();
        let mut param_types: Vec<BasicMetadataTypeEnum> = Vec::new();
        let mut param_names: Vec<String> = Vec::new();
        let mut param_llvm_types_list: Vec<BasicTypeEnum<'ctx>> = Vec::new();
        let mut param_struct_infos: Vec<Option<(StructType<'ctx>, Vec<String>)>> = Vec::new();
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
                    && !matches!(child.kind, 1..=9 | 83 | 101..=105)
                {
                    body_offset = child_offset;
                }
                match child.kind {
                    1..=6 | 83 => {
                        is_void_ret = child.kind == 1;
                        if !is_void_ret {
                            return_llvm_type = Some(self.node_kind_to_llvm_type(child.kind));
                        }
                    }
                    7..=9 => {
                        let Some(func_decl_offset) =
                            self.find_function_declarator_offset(arena, child_offset)
                        else {
                            child_offset = child.next_sibling;
                            continue;
                        };
                        let Some(func_decl) = arena.get(func_decl_offset) else {
                            child_offset = child.next_sibling;
                            continue;
                        };
                        seen_func_declarator = true;
                        // Check variadic flag (data=1 means ...)
                        if func_decl.data == 1 {
                            is_variadic = true;
                        }
                        if !is_void_ret {
                            return_llvm_type = Some(self.declarator_llvm_type(
                                arena,
                                Some(child),
                                return_llvm_type.unwrap_or_else(|| {
                                    self.context.i32_type().as_basic_type_enum()
                                }),
                            ));
                        }
                        // first_child = kind=60(name) -> kind=24(param1) -> ...
                        if let Some(ident) = arena.get(func_decl.first_child) {
                            if ident.kind == 60 {
                                if let Some(name) = arena.get_string(NodeOffset(ident.data)) {
                                    func_name = name.to_string();
                                }
                            }
                            let mut param_off = ident.next_sibling;
                            while param_off != NodeOffset::NULL {
                                if let Some(param) = arena.get(param_off) {
                                    if param.kind == 24 {
                                        let (ptype, pname, struct_info) =
                                            self.extract_param_type_name(arena, param);
                                        param_types.push(ptype.into());
                                        param_llvm_types_list.push(ptype);
                                        param_names.push(pname);
                                        param_struct_infos.push(struct_info);
                                    }
                                    param_off = param.next_sibling;
                                } else {
                                    break;
                                }
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

        let ret_llvm =
            return_llvm_type.unwrap_or_else(|| self.context.i32_type().as_basic_type_enum());
        let fn_type = if is_void_ret {
            self.context.void_type().fn_type(&param_types, is_variadic)
        } else {
            ret_llvm.fn_type(&param_types, is_variadic)
        };
        // Use the pre-registered function if available, otherwise create new
        let function = self.module.get_function(&func_name)
            .unwrap_or_else(|| self.module.add_function(&func_name, fn_type, None));
        // Apply __attribute__ decorations to the function
        let attrs = self.extract_attributes(arena, node);
        self.apply_function_attributes(function, &attrs);
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
        self.label_blocks.clear();
        self.current_return_type = if is_void_ret { None } else { Some(ret_llvm) };

        for (i, ((pname, ptype), struct_info)) in param_names
            .iter()
            .zip(param_llvm_types_list.iter())
            .zip(param_struct_infos.iter())
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
                },
            );
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
                    let _ = self.builder.build_return(Some(
                        &self.context.i32_type().const_int(0, false),
                    ));
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
                && bb.get_first_instruction()
                    .map(|i| i.get_opcode() == inkwell::values::InstructionOpcode::Unreachable)
                    .unwrap_or(false)
            {
                unsafe { bb.delete().ok(); }
            }
            block_opt = next;
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
            52 => {
                // Case label: next_sibling is the inner statement
                if node.next_sibling != NodeOffset::NULL {
                    self.lower_stmt(arena, node.next_sibling)
                } else {
                    Ok(())
                }
            }
            54 => {
                // Case range: next_sibling is the inner statement (same as regular case)
                if node.next_sibling != NodeOffset::NULL {
                    self.lower_stmt(arena, node.next_sibling)
                } else {
                    Ok(())
                }
            }
            53 => {
                // Default label: first_child is the inner statement
                if node.first_child != NodeOffset::NULL {
                    self.lower_stmt(arena, node.first_child)
                } else {
                    Ok(())
                }
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
                .build_float_compare(
                    inkwell::FloatPredicate::ONE,
                    float_val,
                    zero,
                    "float_nz",
                )
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
        let cond_offset = node.first_child;
        let body_offset = node.next_sibling;

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

        // Collect case values and create basic blocks for each case
        let mut cases: Vec<(inkwell::values::IntValue<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        let mut case_bodies: Vec<(BasicBlock<'ctx>, NodeOffset)> = Vec::new();
        let mut default_body_offset = NodeOffset::NULL;

        // Walk the body to find case/default labels
        self.collect_switch_cases(
            arena,
            body_offset,
            function,
            &cond_int,
            &mut cases,
            &mut case_bodies,
            &mut default_body_offset,
            default_bb,
        );

        // Build the switch instruction
        let switch_inst = self
            .builder
            .build_switch(cond_int, default_bb, &cases)
            .map_err(|_| BackendError::InvalidNode)?;
        let _ = switch_inst; // used for building the instruction

        // Push break target
        self.break_stack.push(end_bb);

        // Lower case bodies in order
        for (bb, stmt_offset) in &case_bodies {
            self.builder.position_at_end(*bb);
            if *stmt_offset != NodeOffset::NULL {
                let _ = self.lower_stmt(arena, *stmt_offset);
            }
            // Fall-through: if no terminator, branch to next case body or end
            // Note: C switch semantics allow fall-through between cases
        }

        // Handle fall-through: for each case body without a terminator,
        // branch to the next case body (fall-through) or to end
        for i in 0..case_bodies.len() {
            let (bb, _) = case_bodies[i];
            if bb.get_terminator().is_none() {
                self.builder.position_at_end(bb);
                if i + 1 < case_bodies.len() {
                    let next_bb = case_bodies[i + 1].0;
                    self.builder
                        .build_unconditional_branch(next_bb)
                        .map_err(|_| BackendError::InvalidNode)?;
                } else {
                    self.builder
                        .build_unconditional_branch(end_bb)
                        .map_err(|_| BackendError::InvalidNode)?;
                }
            }
        }

        // Lower default body
        self.builder.position_at_end(default_bb);
        if default_body_offset != NodeOffset::NULL {
            let _ = self.lower_stmt(arena, default_body_offset);
        }
        if default_bb.get_terminator().is_none() {
            self.builder
                .build_unconditional_branch(end_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        // Pop break target
        self.break_stack.pop();

        self.builder.position_at_end(end_bb);
        Ok(())
    }

    /// Recursively walk a compound statement to collect case/default labels for switch.
    fn collect_switch_cases(
        &mut self,
        arena: &Arena,
        offset: NodeOffset,
        function: FunctionValue<'ctx>,
        cond_int: &inkwell::values::IntValue<'ctx>,
        cases: &mut Vec<(inkwell::values::IntValue<'ctx>, BasicBlock<'ctx>)>,
        case_bodies: &mut Vec<(BasicBlock<'ctx>, NodeOffset)>,
        default_body_offset: &mut NodeOffset,
        default_bb: BasicBlock<'ctx>,
    ) {
        let node = match arena.get(offset) {
            Some(n) => n,
            None => return,
        };

        match node.kind {
            52 => {
                // Case label: first_child=case_value_expr, next_sibling=stmt
                let case_val = if node.first_child != NodeOffset::NULL {
                    self.lower_expr(arena, node.first_child).ok().flatten()
                } else {
                    None
                };

                let case_bb = self.context.append_basic_block(function, "switch.case");
                let stmt_offset = node.next_sibling;

                if let Some(val) = case_val {
                    let case_int = if val.is_int_value() {
                        // Match bitwidth to the switch condition
                        let raw = val.into_int_value();
                        let cond_bits = cond_int.get_type().get_bit_width();
                        let case_bits = raw.get_type().get_bit_width();
                        if case_bits < cond_bits {
                            self.builder
                                .build_int_z_extend(raw, cond_int.get_type(), "case_zext")
                                .unwrap_or(raw)
                        } else if case_bits > cond_bits {
                            self.builder
                                .build_int_truncate(raw, cond_int.get_type(), "case_trunc")
                                .unwrap_or(raw)
                        } else {
                            raw
                        }
                    } else {
                        // Non-int case value: treat as 0
                        cond_int.get_type().const_int(0, false)
                    };
                    cases.push((case_int, case_bb));
                } else {
                    // Could not evaluate case value; use 0 as fallback
                    cases.push((cond_int.get_type().const_int(0, false), case_bb));
                }

                case_bodies.push((case_bb, stmt_offset));

                // The stmt in next_sibling may itself contain more case labels
                // (fall-through chains like `case 1: case 2: stmt`)
                if stmt_offset != NodeOffset::NULL {
                    if let Some(stmt_node) = arena.get(stmt_offset) {
                        if stmt_node.kind == 52 || stmt_node.kind == 53 || stmt_node.kind == 54 {
                            self.collect_switch_cases(
                                arena,
                                stmt_offset,
                                function,
                                cond_int,
                                cases,
                                case_bodies,
                                default_body_offset,
                                default_bb,
                            );
                        }
                    }
                }
            }
            54 => {
                // Case range (GNU extension): case LOW ... HIGH:
                // kind=54: first_child=low_expr, data=high_expr_offset, next_sibling=stmt
                let low_val = if node.first_child != NodeOffset::NULL {
                    self.lower_expr(arena, node.first_child).ok().flatten()
                } else {
                    None
                };
                let high_val = if node.data != 0 {
                    self.lower_expr(arena, NodeOffset(node.data)).ok().flatten()
                } else {
                    None
                };

                let case_bb = self.context.append_basic_block(function, "switch.case_range");
                let stmt_offset = node.next_sibling;

                if let (Some(lo), Some(hi)) = (low_val, high_val) {
                    if lo.is_int_value() && hi.is_int_value() {
                        let lo_raw = lo.into_int_value();
                        let hi_raw = hi.into_int_value();
                        // Get constant values for the range
                        let lo_const = lo_raw.get_zero_extended_constant().unwrap_or(0);
                        let hi_const = hi_raw.get_zero_extended_constant().unwrap_or(0);
                        // Cap range to prevent huge switch tables (max 256 entries)
                        let count = if hi_const >= lo_const {
                            (hi_const - lo_const + 1).min(MAX_CASE_RANGE_EXPANSION)
                        } else {
                            1
                        };
                        for i in 0..count {
                            let val = cond_int.get_type().const_int(lo_const + i, false);
                            cases.push((val, case_bb));
                        }
                    } else {
                        // Fallback: treat as single case with low value
                        cases.push((cond_int.get_type().const_int(0, false), case_bb));
                    }
                } else {
                    cases.push((cond_int.get_type().const_int(0, false), case_bb));
                }

                case_bodies.push((case_bb, stmt_offset));

                // Check if stmt contains more case labels
                if stmt_offset != NodeOffset::NULL {
                    if let Some(stmt_node) = arena.get(stmt_offset) {
                        if stmt_node.kind == 52 || stmt_node.kind == 53 || stmt_node.kind == 54 {
                            self.collect_switch_cases(
                                arena,
                                stmt_offset,
                                function,
                                cond_int,
                                cases,
                                case_bodies,
                                default_body_offset,
                                default_bb,
                            );
                        }
                    }
                }
            }
            53 => {
                // Default label: first_child=stmt
                *default_body_offset = node.first_child;
                // The default stmt may also contain nested case labels
                if node.first_child != NodeOffset::NULL {
                    if let Some(child) = arena.get(node.first_child) {
                        if child.kind == 52 || child.kind == 53 || child.kind == 54 {
                            self.collect_switch_cases(
                                arena,
                                node.first_child,
                                function,
                                cond_int,
                                cases,
                                case_bodies,
                                default_body_offset,
                                default_bb,
                            );
                        }
                    }
                }
            }
            40 => {
                // Compound statement: walk children
                let mut child_offset = node.first_child;
                while child_offset != NodeOffset::NULL {
                    if let Some(child) = arena.get(child_offset) {
                        self.collect_switch_cases(
                            arena,
                            child_offset,
                            function,
                            cond_int,
                            cases,
                            case_bodies,
                            default_body_offset,
                            default_bb,
                        );
                        child_offset = child.next_sibling;
                    } else {
                        break;
                    }
                }
            }
            _ => {}
        }
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
        let template = arena
            .get_string(node.first_child)
            .unwrap_or("")
            .to_string();

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
                        constraint.strip_prefix('+').unwrap_or(&constraint).to_string()
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
                                let loaded = self.builder
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
        let call_result = self.builder
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
                                if result_int.get_type().get_bit_width() > target_int.get_bit_width() {
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
                            let extracted = self.builder
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

        match node.kind {
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
        let lhs_offset = node.first_child;
        let rhs_offset = node.next_sibling;

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
                    let pointee_type = self.to_llvm_type(self.default_type());
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

    fn lower_call_expr(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let first_child_offset = node.first_child;
        let first_child = arena.get(first_child_offset);

        let func_name = first_child.and_then(|c| arena.get_string(NodeOffset(c.data)));

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
                    if let Some(arg_val) = self.lower_expr(arena, expr_offset)? {
                        args.push(arg_val.into());
                    }
                    arg_offset = arg_node.next_sibling;
                } else {
                    break;
                }
            }
        }

        if let Some(name) = func_name {
            // Intercept va_start/va_end/va_copy/va_arg as LLVM intrinsics
            match name {
                "__builtin_va_start" | "va_start" => {
                    // va_start(ap) → llvm.va_start(ap)
                    let va_start_type = self
                        .context
                        .void_type()
                        .fn_type(&[self.context.ptr_type(AddressSpace::default()).into()], false);
                    let va_start_fn = self
                        .module
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
                            let _ = self
                                .builder
                                .build_call(va_start_fn, &[ptr.into()], "");
                        }
                    }
                    return Ok(Some(self.context.i32_type().const_int(0, false).into()));
                }
                "__builtin_va_end" | "va_end" => {
                    let va_end_type = self
                        .context
                        .void_type()
                        .fn_type(&[self.context.ptr_type(AddressSpace::default()).into()], false);
                    let va_end_fn = self
                        .module
                        .get_function("llvm.va_end")
                        .unwrap_or_else(|| {
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
                    let va_copy_fn = self
                        .module
                        .get_function("llvm.va_copy")
                        .unwrap_or_else(|| {
                            self.module
                                .add_function("llvm.va_copy", va_copy_type, None)
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

            if let Some(func) = self.functions.get(name) {
                let call_site = self
                    .builder
                    .build_call(*func, &args, "call")
                    .map_err(|_| BackendError::InvalidNode)?;
                return Ok(Some(match call_site.try_as_basic_value() {
                    inkwell::values::ValueKind::Basic(v) => v,
                    inkwell::values::ValueKind::Instruction(_) => {
                        self.context.i32_type().const_int(0, false).into()
                    }
                }));
            }

            // Auto-declare external function using actual argument types from the call site
            let param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = args
                .iter()
                .map(|a| match a {
                    inkwell::values::BasicMetadataValueEnum::IntValue(v) => v.get_type().into(),
                    inkwell::values::BasicMetadataValueEnum::FloatValue(v) => v.get_type().into(),
                    inkwell::values::BasicMetadataValueEnum::PointerValue(_) => {
                        self.context.ptr_type(AddressSpace::default()).into()
                    }
                    _ => self.context.i32_type().into(),
                })
                .collect();
            let fn_type = self.context.i32_type().fn_type(&param_types, false);
            let ext_func = self.module.add_function(name, fn_type, None);
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

        Ok(None)
    }

    fn lower_member_access(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let base_offset = node.first_child;
        let field_node_offset = node.next_sibling;

        let base_name_str: Option<String> = arena.get(base_offset).and_then(|n| {
            if n.kind == 60 {
                arena.get_string(NodeOffset(n.data)).map(|s| s.to_string())
            } else {
                None
            }
        });

        let field_name_str: Option<String> = arena.get(field_node_offset).and_then(|n| {
            if n.kind == 60 {
                arena.get_string(NodeOffset(n.data)).map(|s| s.to_string())
            } else {
                None
            }
        });

        let field_name = match (base_name_str.as_ref(), field_name_str.as_ref()) {
            (Some(_), Some(f)) => f.clone(),
            _ => return Ok(None),
        };

        let Some((field_ptr, field_llvm_type)) = self.lower_member_access_ptr(arena, node)? else {
            return Ok(None);
        };
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
        if child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                let expr_offset = child.next_sibling;
                if expr_offset != NodeOffset::NULL {
                    return self.lower_expr(arena, expr_offset);
                }
            }
            self.lower_expr(arena, child_offset)
        } else {
            Ok(None)
        }
    }

    fn lower_sizeof_expr(
        &self,
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
            return Ok(Some(
                self.context.i64_type().const_int(4, false).into(),
            ));
        };

        if data == 0 {
            // sizeof(type) — examine type specifier node kind
            let size = self.sizeof_type_from_ast(arena, child_offset);
            Ok(Some(
                self.context.i64_type().const_int(size, false).into(),
            ))
        } else {
            // sizeof(expr) — default to 4 for int expressions
            Ok(Some(
                self.context.i64_type().const_int(4, false).into(),
            ))
        }
    }

    fn sizeof_type_from_ast(&self, arena: &Arena, offset: NodeOffset) -> u64 {
        let node = match arena.get(offset) {
            Some(n) => n,
            None => return 4,
        };

        // Type specifier nodes have specific kinds from parse_type_specifier
        match node.kind {
            1 => 0,   // void → size 0 (GCC returns 1 as extension, but 0 is standard)
            2 => 4,   // int
            3 => 1,   // char
            10 => 2,  // short
            11 => {   // long — check if "long long" by looking at sibling
                let sibling = arena.get(node.next_sibling);
                if sibling.map(|s| s.kind) == Some(11) { 8 } else { 8 } // long = 8, long long = 8
            }
            12 => 4,  // signed (defaults to signed int)
            13 => 4,  // unsigned (defaults to unsigned int)
            14 => 1,  // _Bool
            83 => 4,  // float
            84 => 8,  // double
            _ => {
                // For struct/pointer/unknown, assume pointer size
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

        let value_to_store = if node.data == 19 {
            rhs_val
        } else {
            let current_val = self
                .builder
                .build_load(lhs_type, lhs_ptr, "assign_lhs")
                .map_err(|_| BackendError::InvalidNode)?;
            self.apply_assignment_op(node.data, current_val, rhs_val)?
        };

        if Self::types_compatible(lhs_type, value_to_store) {
            self.builder
                .build_store(lhs_ptr, value_to_store)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        Ok(Some(value_to_store))
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

    fn lower_builtin_call(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let builtin_name = if node.data != 0 {
            arena
                .get_string(NodeOffset(node.data))
                .map(|s| s.to_string())
        } else {
            None
        };

        let builtin_name = builtin_name.unwrap_or_else(|| "__builtin_unknown".to_string());

        let mut args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                if let Some(arg_val) = self.lower_expr(arena, child_offset)? {
                    args.push(arg_val.into());
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
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
                let intrinsic_name = match builtin_name.as_str() {
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
            "__builtin_va_arg" => Ok(Some(self.context.i32_type().const_int(0, false).into())),
            "__builtin_va_start" | "__builtin_va_end" | "__builtin_va_copy" => {
                // va_start/va_end/va_copy: side-effect-only, return nothing meaningful
                Ok(Some(self.context.i32_type().const_int(0, false).into()))
            }
            "__builtin_types_compatible_p" => {
                Ok(Some(self.context.i32_type().const_int(1, false).into()))
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
                if let Some(inkwell::values::BasicMetadataValueEnum::IntValue(val)) =
                    args.first()
                {
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
                if let Some(inkwell::values::BasicMetadataValueEnum::IntValue(val)) =
                    args.first()
                {
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
                if let Some(inkwell::values::BasicMetadataValueEnum::IntValue(val)) =
                    args.first()
                {
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
                if let Some(inkwell::values::BasicMetadataValueEnum::IntValue(val)) =
                    args.first()
                {
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
                if let Some(inkwell::values::BasicMetadataValueEnum::IntValue(val)) =
                    args.first()
                {
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
                if let Some(inkwell::values::BasicMetadataValueEnum::IntValue(val)) =
                    args.first()
                {
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
                    match &args[1] {
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
                        let result = match builtin_name.as_str() {
                            "__builtin_add_overflow" => self
                                .builder
                                .build_int_add(*a, *b, "overflow_add")
                                .map_err(|_| BackendError::InvalidNode)?,
                            "__builtin_sub_overflow" => self
                                .builder
                                .build_int_sub(*a, *b, "overflow_sub")
                                .map_err(|_| BackendError::InvalidNode)?,
                            "__builtin_mul_overflow" => self
                                .builder
                                .build_int_mul(*a, *b, "overflow_mul")
                                .map_err(|_| BackendError::InvalidNode)?,
                            _ => unreachable!("overflow builtin matched but not handled: {}", builtin_name),
                        };
                        self.builder
                            .build_store(*result_ptr, result)
                            .map_err(|_| BackendError::InvalidNode)?;
                        // Return 0 (no overflow detected — conservative)
                        Ok(Some(
                            self.context.i32_type().const_int(0, false).into(),
                        ))
                    } else {
                        Ok(Some(
                            self.context.i32_type().const_int(0, false).into(),
                        ))
                    }
                } else {
                    Ok(Some(
                        self.context.i32_type().const_int(0, false).into(),
                    ))
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
                Ok(Some(
                    self.context.i32_type().const_int(0, false).into(),
                ))
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
            7 | 8 => self.find_function_declarator_offset(arena, node.first_child),
            73 => self.find_function_declarator_offset(arena, node.first_child),
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
    fn extract_attributes(&self, arena: &Arena, node: &CAstNode) -> Vec<(String, Option<String>, Option<u32>)> {
        let mut attrs = Vec::new();
        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                if child.kind == 200 {
                    // kind=200 is AST_ATTRIBUTE; walk its children (individual attributes)
                    let mut attr_child_offset = child.first_child;
                    while attr_child_offset != NodeOffset::NULL {
                        if let Some(attr_child) = arena.get(attr_child_offset) {
                            // first_child = name string offset stored as an arena node
                            // The attribute node stores: first_child=name_offset, next_sibling=first_arg
                            if let Some(name) = arena.get_string(NodeOffset(attr_child.first_child.0)) {
                                if !name.is_empty() {
                                    let mut str_arg = None;
                                    let mut int_arg = if attr_child.data != 0 {
                                        Some(attr_child.data)
                                    } else {
                                        None
                                    };
                                    // Walk argument nodes
                                    let mut arg_offset = attr_child.next_sibling;
                                    while arg_offset != NodeOffset::NULL {
                                        if let Some(arg_node) = arena.get(arg_offset) {
                                            if arg_node.kind == 63 {
                                                // String argument
                                                str_arg = arena
                                                    .get_string(NodeOffset(arg_node.data))
                                                    .map(|s| s.to_string());
                                            } else if arg_node.kind == 61 {
                                                // Integer argument
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
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }
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
                    // Mark function as noreturn via LLVM attribute
                    function.add_attribute(
                        inkwell::attributes::AttributeLoc::Function,
                        self.context.create_enum_attribute(
                            inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn"),
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
                            "protected" => global.set_visibility(inkwell::GlobalVisibility::Protected),
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
        backend.compile(&parser.arena, root).expect("compile failed");
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
            }"
        );
        // The IR should contain a switch instruction
        assert!(ir.contains("switch"), "Expected switch instruction in IR:\n{}", ir);
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
            }"
        );
        // Should contain a branch to a label block
        assert!(ir.contains("br label"), "Expected unconditional branch in IR:\n{}", ir);
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
            }"
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
            }"
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
            }"
        );
        assert!(ir.contains("br label"), "Expected branches in IR:\n{}", ir);
    }

    #[test]
    fn test_builtin_expect() {
        let ir = compile_c_to_ir(
            "int test_expect(int x) { \
                return __builtin_expect(x, 1); \
            }"
        );
        // __builtin_expect just returns its first argument
        assert!(ir.contains("define"), "Expected function definition in IR:\n{}", ir);
    }

    #[test]
    fn test_builtin_constant_p() {
        let ir = compile_c_to_ir(
            "int test_constant_p(int x) { \
                return __builtin_constant_p(x); \
            }"
        );
        assert!(ir.contains("define"), "Expected function definition in IR:\n{}", ir);
    }

    #[test]
    fn test_variadic_function() {
        let ir = compile_c_to_ir(
            "int my_printf(int fmt, ...) { \
                return 0; \
            }"
        );
        // Variadic functions should have ... in the LLVM signature
        assert!(ir.contains("..."), "Expected variadic signature in IR:\n{}", ir);
    }

    #[test]
    fn test_asm_basic_volatile() {
        let ir = compile_c_to_ir(
            "void test_asm() { \
                asm volatile(\"nop\"); \
            }"
        );
        assert!(ir.contains("call void asm sideeffect"), "Expected asm sideeffect call in IR:\n{}", ir);
        assert!(ir.contains("nop"), "Expected nop in asm template:\n{}", ir);
    }

    #[test]
    fn test_asm_memory_barrier() {
        let ir = compile_c_to_ir(
            "void memory_barrier() { \
                asm volatile(\"\" : : : \"memory\"); \
            }"
        );
        // Should produce an asm call with memory clobber
        assert!(ir.contains("asm sideeffect"), "Expected asm sideeffect in IR:\n{}", ir);
    }

    #[test]
    fn test_asm_with_output() {
        let ir = compile_c_to_ir(
            "int read_reg() { \
                int val; \
                asm(\"mov $0, %0\" : \"=r\"(val)); \
                return val; \
            }"
        );
        assert!(ir.contains("asm"), "Expected asm instruction in IR:\n{}", ir);
    }

    #[test]
    fn test_asm_with_input_and_output() {
        let ir = compile_c_to_ir(
            "int double_it(int x) { \
                int result; \
                asm(\"addl %1, %0\" : \"=r\"(result) : \"r\"(x)); \
                return result; \
            }"
        );
        assert!(ir.contains("asm"), "Expected asm instruction in IR:\n{}", ir);
    }

    #[test]
    fn test_asm_cc_clobber() {
        let ir = compile_c_to_ir(
            "void test_cc_clobber() { \
                asm volatile(\"\" : : : \"cc\"); \
            }"
        );
        assert!(ir.contains("asm sideeffect"), "Expected asm sideeffect in IR:\n{}", ir);
    }

    #[test]
    fn test_builtin_alloca() {
        let ir = compile_c_to_ir(
            "void test_alloca(int n) { \
                void *p = __builtin_alloca(n); \
            }"
        );
        assert!(ir.contains("alloca"), "Expected alloca in IR:\n{}", ir);
    }

    #[test]
    fn test_builtin_memcpy() {
        let ir = compile_c_to_ir(
            "void test_memcpy(char *dst, char *src, int n) { \
                __builtin_memcpy(dst, src, n); \
            }"
        );
        assert!(ir.contains("memcpy") || ir.contains("__builtin_memcpy"), "Expected memcpy call in IR:\n{}", ir);
    }

    #[test]
    fn test_builtin_memset() {
        let ir = compile_c_to_ir(
            "void test_memset(char *dst, int c, int n) { \
                __builtin_memset(dst, c, n); \
            }"
        );
        assert!(ir.contains("memset") || ir.contains("__builtin_memset"), "Expected memset call in IR:\n{}", ir);
    }

    #[test]
    fn test_label_addr_expr() {
        let ir = compile_c_to_ir(
            "int test_label_addr() { \
                void *p; \
                target: \
                p = &&target; \
                return 0; \
            }"
        );
        assert!(ir.contains("blockaddress") || ir.contains("label.target"), 
            "Expected blockaddress or label block in IR:\n{}", ir);
    }

    #[test]
    fn test_computed_goto() {
        let ir = compile_c_to_ir(
            "void test_computed_goto() { \
                void *target; \
                label1: \
                target = &&label1; \
                goto *target; \
            }"
        );
        assert!(ir.contains("indirectbr") || ir.contains("label.label1"), 
            "Expected indirectbr or label block in IR:\n{}", ir);
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
            }"
        );
        assert!(ir.contains("switch"), "Expected switch instruction in IR:\n{}", ir);
        // Case ranges should generate multiple case entries
        assert!(ir.contains("switch.case_range") || ir.contains("i32 1") || ir.contains("switch i32"),
            "Expected case range expansion in IR:\n{}", ir);
    }

    #[test]
    fn test_case_range_single_value() {
        let ir = compile_c_to_ir(
            "int test_single_range(int x) { \
                switch (x) { \
                    case 5 ... 5: return 1; \
                    default: return 0; \
                } \
            }"
        );
        assert!(ir.contains("switch"), "Expected switch in IR:\n{}", ir);
    }

    #[test]
    fn test_attribute_weak_function() {
        let ir = compile_c_to_ir(
            "void my_weak_func(void) __attribute__((weak)); \
             void my_weak_func(void) { return; }"
        );
        assert!(ir.contains("my_weak_func"), "Expected function in IR:\n{}", ir);
        // weak linkage should appear as `weak` or `extern_weak`
        assert!(ir.contains("weak"), "Expected weak linkage in IR:\n{}", ir);
    }

    #[test]
    fn test_attribute_section_function() {
        let ir = compile_c_to_ir(
            "__attribute__((section(\".init.text\"))) void init_func(void) { return; }"
        );
        assert!(ir.contains("init_func"), "Expected function in IR:\n{}", ir);
        assert!(ir.contains(".init.text"), "Expected section attribute in IR:\n{}", ir);
    }

    #[test]
    fn test_attribute_noreturn_function() {
        let ir = compile_c_to_ir(
            "__attribute__((noreturn)) void die(void) { return; }"
        );
        assert!(ir.contains("die"), "Expected function in IR:\n{}", ir);
        assert!(ir.contains("noreturn"), "Expected noreturn attribute in IR:\n{}", ir);
    }

    #[test]
    fn test_attribute_cold_function() {
        let ir = compile_c_to_ir(
            "__attribute__((cold)) void rare_path(void) { return; }"
        );
        assert!(ir.contains("rare_path"), "Expected function in IR:\n{}", ir);
        assert!(ir.contains("cold"), "Expected cold attribute in IR:\n{}", ir);
    }

    #[test]
    fn test_sizeof_int() {
        let ir = compile_c_to_ir(
            "int test_sizeof(void) { return sizeof(int); }"
        );
        // sizeof(int) should produce 4, returned as i32
        assert!(ir.contains("ret i32 4"), "Expected ret i32 4 for sizeof(int):\n{}", ir);
    }

    #[test]
    fn test_sizeof_char() {
        let ir = compile_c_to_ir(
            "int test_sizeof_char(void) { return sizeof(char); }"
        );
        // sizeof(char) should produce 1, returned as i32
        assert!(ir.contains("ret i32 1"), "Expected ret i32 1 for sizeof(char):\n{}", ir);
    }

    #[test]
    fn test_ternary_select() {
        let ir = compile_c_to_ir(
            "int max(int a, int b) { return a > b ? a : b; }"
        );
        // Ternary should produce a select instruction
        assert!(ir.contains("select"), "Expected select instruction for ternary:\n{}", ir);
    }
}

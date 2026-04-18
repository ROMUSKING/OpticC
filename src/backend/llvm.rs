use crate::arena::{Arena, CAstNode, NodeFlags, NodeOffset};
use crate::types::{CType, TypeId, TypeSystem};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, StructType};
use inkwell::values::{BasicValue, BasicValueEnum, FunctionValue, PointerValue};
use inkwell::AddressSpace;
use std::collections::HashMap;

pub struct LlvmBackend<'ctx, 'types> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    variables: HashMap<String, VariableBinding<'ctx>>,
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
            functions: HashMap::new(),
            vectorization_hints: VectorizationHints::default(),
            types: None,
            type_cache: HashMap::new(),
            struct_fields: HashMap::new(),
            struct_tag_types: HashMap::new(),
            struct_tag_fields: HashMap::new(),
            pointer_struct_types: HashMap::new(),
            current_return_type: None,
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
            functions: HashMap::new(),
            vectorization_hints: VectorizationHints::default(),
            types: Some(types),
            type_cache: HashMap::new(),
            struct_fields: HashMap::new(),
            struct_tag_types: HashMap::new(),
            struct_tag_fields: HashMap::new(),
            pointer_struct_types: HashMap::new(),
            current_return_type: None,
        }
    }

    pub fn set_vectorization_hints(&mut self, hints: VectorizationHints) {
        self.vectorization_hints = hints;
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

        let mut child_offset = offset;
        while child_offset != NodeOffset::NULL {
            if let Some(node) = arena.get(child_offset) {
                match node.kind {
                    1..=9 | 83 => {}
                    20 => self.lower_decl(arena, node)?,
                    22 => self.lower_func_decl(arena, node)?,
                    23 => self.lower_func_def(arena, node)?,
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

    fn lower_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                match child.kind {
                    21 => self.lower_var_decl(arena, &child)?,
                    22 => self.lower_func_decl(arena, &child)?,
                    23 => self.lower_func_def(arena, &child)?,
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
                                .builder
                                .build_alloca(actual_alloca_type, &var_name)
                                .map_err(|_| BackendError::InvalidNode)?;
                            if let Some((struct_type, field_names)) = struct_info.clone() {
                                self.struct_fields
                                    .insert(var_name.clone(), field_names.clone());
                                if actual_alloca_type.is_pointer_type() {
                                    self.pointer_struct_types
                                        .insert(var_name.clone(), struct_type);
                                }
                            }
                            self.variables.insert(
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
                                .builder
                                .build_alloca(actual_alloca_type, &var_name)
                                .map_err(|_| BackendError::InvalidNode)?;
                            if let Some((struct_type, field_names)) = struct_info.clone() {
                                self.struct_fields
                                    .insert(var_name.clone(), field_names.clone());
                                if actual_alloca_type.is_pointer_type() {
                                    self.pointer_struct_types
                                        .insert(var_name.clone(), struct_type);
                                }
                            }
                            self.variables.insert(
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
            None => { eprintln!("[DEBUG ptr] base_name not found"); return Ok(None); },
        };

        let field_name = match arena.get(field_offset).and_then(|n| {
            if n.kind == 60 {
                arena.get_string(NodeOffset(n.data)).map(|s| s.to_string())
            } else {
                None
            }
        }) {
            Some(name) => name,
            None => { eprintln!("[DEBUG ptr] field_name not found"); return Ok(None); },
        };

        eprintln!("[DEBUG ptr] base_name={:?}, field_name={:?}, node.data={}", base_name, field_name, node.data);
        eprintln!("[DEBUG ptr] struct_fields keys: {:?}", self.struct_fields.keys().collect::<Vec<_>>());
        eprintln!("[DEBUG ptr] pointer_struct_types keys: {:?}", self.pointer_struct_types.keys().collect::<Vec<_>>());

        let field_idx = self
            .struct_fields
            .get(&base_name)
            .and_then(|fields| fields.iter().position(|f| f == &field_name))
            .unwrap_or(0) as u32;

        eprintln!("[DEBUG ptr] field_idx={}", field_idx);

        if node.data == 1 {
            let Some(struct_type) = self.pointer_struct_types.get(&base_name).copied() else {
                eprintln!("[DEBUG ptr] pointer_struct_types missing {:?}", base_name);
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

        let base_ptr = match self.lower_expr(arena, node.first_child)? {
            Some(BasicValueEnum::PointerValue(ptr)) => ptr,
            _ => return Ok(None),
        };
        let elem_type = self.context.i32_type().as_basic_type_enum();
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

        let lhs_int = lhs_val.into_int_value();
        let rhs_int = rhs_val.into_int_value();
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

    fn lower_func_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let name = self.find_ident_name(arena, node);
        let func_name = name.as_deref().unwrap_or("func");
        let fn_type = self.context.i32_type().fn_type(&[], false);
        let function = self.module.add_function(func_name, fn_type, None);
        if let Some(n) = name {
            self.functions.insert(n, function);
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
            self.context.void_type().fn_type(&param_types, false)
        } else {
            ret_llvm.fn_type(&param_types, false)
        };
        let function = self.module.add_function(&func_name, fn_type, None);
        self.functions.insert(func_name.clone(), function);

        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);

        self.variables.clear();
        self.struct_fields.clear();
        self.pointer_struct_types.clear();
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
                    self.lower_stmt(arena, stmt_off)?;
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
                } else {
                    let int_type = ret_llvm.into_int_type();
                    self.builder
                        .build_return(Some(&int_type.const_int(0, false)))
                        .map_err(|_| BackendError::InvalidNode)?;
                }
            }
        }

        self.current_return_type = None;
        Ok(())
    }

    fn lower_compound(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                self.lower_stmt(arena, child_offset)?;
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }
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
            46 | 47 => Ok(()),
            50 => Ok(()),
            1..=9 | 83 | 90..=94 | 101..=105 => Ok(()),
            24 => Ok(()),
            25 | 26 => Ok(()),
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
            Ok(val.into_int_value())
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
            let zero = self.context.f64_type().const_float(0.0);
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

        let cond_bb = self.context.append_basic_block(function, "while.cond");
        let body_bb = self.context.append_basic_block(function, "while.body");
        let end_bb = self.context.append_basic_block(function, "while.end");

        self.builder
            .build_unconditional_branch(cond_bb)
            .map_err(|_| BackendError::InvalidNode)?;

        self.builder.position_at_end(cond_bb);
        let cond_val = self
            .lower_expr(arena, cond_offset)?
            .ok_or(BackendError::InvalidNode)?;
        let cond_int = self.coerce_to_bool(cond_val)?;
        self.builder
            .build_conditional_branch(cond_int, body_bb, end_bb)
            .map_err(|_| BackendError::InvalidNode)?;

        self.builder.position_at_end(body_bb);
        self.lower_stmt(arena, body_offset)?;
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
        let body_bb = self.context.append_basic_block(function, "for.body");
        let end_bb = self.context.append_basic_block(function, "for.end");

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
            self.lower_stmt(arena, body_offset)?;
        }
        if incr_offset != NodeOffset::NULL {
            let _ = self.lower_expr(arena, incr_offset)?;
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

        self.builder.position_at_end(end_bb);
        Ok(())
    }

    fn lower_return_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        if node.first_child != NodeOffset::NULL {
            if let Some(val) = self.lower_expr(arena, node.first_child)? {
                if let Some(BasicTypeEnum::PointerType(ptr_type)) = self.current_return_type {
                    if val.is_pointer_value() {
                        let ptr_val = val.into_pointer_value();
                        self.builder
                            .build_return(Some(&ptr_val))
                            .map_err(|_| BackendError::InvalidNode)?;
                    } else if val.is_int_value()
                        && val.into_int_value().get_zero_extended_constant() == Some(0)
                    {
                        self.builder
                            .build_return(Some(&ptr_type.const_null()))
                            .map_err(|_| BackendError::InvalidNode)?;
                    } else {
                        return Err(BackendError::InvalidNode);
                    }
                } else if val.is_int_value() {
                    let int_val = val.into_int_value();
                    self.builder
                        .build_return(Some(&int_val))
                        .map_err(|_| BackendError::InvalidNode)?;
                } else if val.is_float_value() {
                    let float_val = val.into_float_value();
                    self.builder
                        .build_return(Some(&float_val))
                        .map_err(|_| BackendError::InvalidNode)?;
                } else {
                    return Ok(());
                }
            } else {
                if let Some(BasicTypeEnum::PointerType(ptr_type)) = self.current_return_type {
                    self.builder
                        .build_return(Some(&ptr_type.const_null()))
                        .map_err(|_| BackendError::InvalidNode)?;
                } else {
                    self.builder
                        .build_return(Some(&self.context.i32_type().const_int(0, false)))
                        .map_err(|_| BackendError::InvalidNode)?;
                }
            }
        } else {
            self.builder
                .build_return(None)
                .map_err(|_| BackendError::InvalidNode)?;
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
            63 | 81 => self.lower_string_const(&node),
            64 => self.lower_binop(arena, &node),
            65 => self.lower_unop(arena, &node),
            66 => self.lower_cond_expr(arena, &node),
            67 => self.lower_call_expr(arena, &node),
            68 => self.lower_array_index(arena, &node),
            69 => self.lower_member_access(arena, &node),
            70 => self.lower_cast_expr(arena, &node),
            71 => self.lower_sizeof_expr(&node),
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
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let byte = node.data as u8;
        let string_val = self.context.const_string(&[byte], true);
        let global =
            self.module
                .add_global(string_val.get_type(), Some(AddressSpace::default()), "str");
        global.set_initializer(&string_val);
        global.set_constant(true);
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

        eprintln!("[DEBUG binop] data={}, lhs_offset={:?}, rhs_offset={:?}", node.data, lhs_offset, rhs_offset);
        if let Some(ln) = arena.get(lhs_offset) {
            eprintln!("[DEBUG binop]   lhs kind={}, data={}", ln.kind, ln.data);
        } else {
            eprintln!("[DEBUG binop]   lhs -> None!");
        }
        if let Some(rn) = arena.get(rhs_offset) {
            eprintln!("[DEBUG binop]   rhs kind={}, data={}", rn.kind, rn.data);
        } else {
            eprintln!("[DEBUG binop]   rhs -> None!");
        }

        let lhs_val = self
            .lower_expr(arena, lhs_offset)?;
        eprintln!("[DEBUG binop]   lhs_val = {:?}", lhs_val.as_ref().map(|v| v.get_type().to_string()));
        let lhs_val = lhs_val.ok_or(BackendError::InvalidNode)?;
        let rhs_val = self
            .lower_expr(arena, rhs_offset)?;
        eprintln!("[DEBUG binop]   rhs_val = {:?}", rhs_val.as_ref().map(|v| v.get_type().to_string()));
        let rhs_val = rhs_val.ok_or(BackendError::InvalidNode)?;

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
                    let zero = self.context.i32_type().const_int(0, false);
                    self.builder
                        .build_int_compare(inkwell::IntPredicate::EQ, int_op, zero, "lnot")
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
        let cond_offset = node.first_child;
        let then_offset = if let Some(c) = arena.get(cond_offset) {
            c.next_sibling
        } else {
            NodeOffset::NULL
        };
        let else_offset = if let Some(c) = arena.get(then_offset) {
            c.next_sibling
        } else {
            NodeOffset::NULL
        };

        let cond_val = self.lower_expr(arena, cond_offset)?;
        let then_val = self.lower_expr(arena, then_offset)?;
        let else_val = self.lower_expr(arena, else_offset)?;

        let _ = else_val;
        Ok(then_val)
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
                    if let Some(arg_val) = self.lower_expr(arena, arg_offset)? {
                        args.push(arg_val.into());
                    }
                    arg_offset = arg_node.next_sibling;
                } else {
                    break;
                }
            }
        }

        if let Some(name) = func_name {
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

            let fn_type = self.context.i32_type().fn_type(&[], true);
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

        eprintln!("[DEBUG member_access] data={}, first_child={:?}, next_sibling={:?}",
            node.data, base_offset, field_node_offset);
        if let Some(bn) = arena.get(base_offset) {
            eprintln!("[DEBUG member_access]   base: kind={}, data={}", bn.kind, bn.data);
            if bn.kind == 60 {
                if let Some(s) = arena.get_string(NodeOffset(bn.data)) {
                    eprintln!("[DEBUG member_access]   base name: {:?}", s);
                }
            }
        }
        if let Some(fn_) = arena.get(field_node_offset) {
            eprintln!("[DEBUG member_access]   field_node: kind={}, data={}", fn_.kind, fn_.data);
            if fn_.kind == 60 {
                if let Some(s) = arena.get_string(NodeOffset(fn_.data)) {
                    eprintln!("[DEBUG member_access]   field name: {:?}", s);
                }
            }
        } else {
            eprintln!("[DEBUG member_access]   field_node -> None for offset {:?}", field_node_offset);
        }

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
            _ => {
                eprintln!("[DEBUG] lower_member_access: base_name={:?}, field_name={:?}, base_offset={:?}, field_node_offset={:?}",
                    base_name_str, field_name_str, base_offset, field_node_offset);
                if let Some(bn) = arena.get(base_offset) {
                    eprintln!("[DEBUG]   base_node: kind={}, data={}, first_child={:?}, next_sibling={:?}",
                        bn.kind, bn.data, bn.first_child, bn.next_sibling);
                }
                if let Some(fn_) = arena.get(field_node_offset) {
                    eprintln!("[DEBUG]   field_node: kind={}, data={}, first_child={:?}, next_sibling={:?}",
                        fn_.kind, fn_.data, fn_.first_child, fn_.next_sibling);
                    if fn_.kind == 60 {
                        if let Some(s) = arena.get_string(NodeOffset(fn_.data)) {
                            eprintln!("[DEBUG]   field_node string: {:?}", s);
                        }
                    }
                } else {
                    eprintln!("[DEBUG]   field_node_offset {:?} -> None!", field_node_offset);
                }
                return Ok(None);
            },
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
        _node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(Some(
            self.context
                .i64_type()
                .const_int(std::mem::size_of::<i32>() as u64, false)
                .into(),
        ))
    }

    fn lower_comma_expr(
        &mut self,
        arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let mut last_val: Option<BasicValueEnum> = None;
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

    fn lower_label_addr(
        &mut self,
        _arena: &Arena,
        node: &CAstNode,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let ptr = self.context.ptr_type(AddressSpace::default()).const_null();
        Ok(Some(ptr.into()))
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
            "__builtin_types_compatible_p" => {
                Ok(Some(self.context.i32_type().const_int(1, false).into()))
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
}

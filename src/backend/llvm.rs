use crate::arena::{Arena, CAstNode, NodeOffset, NodeFlags};
use crate::types::{TypeSystem, TypeId, CType};
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::builder::Builder;
use inkwell::types::{BasicType, BasicTypeEnum, BasicMetadataTypeEnum};
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue, BasicValue};
use inkwell::AddressSpace;
use std::collections::HashMap;

pub struct LlvmBackend<'ctx, 'types> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    variables: HashMap<String, PointerValue<'ctx>>,
    functions: HashMap<String, FunctionValue<'ctx>>,
    vectorization_hints: VectorizationHints,
    types: Option<&'types TypeSystem>,
    type_cache: HashMap<u32, BasicTypeEnum<'ctx>>,
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
        }
    }

    pub fn with_types(context: &'ctx Context, module_name: &str, types: &'types TypeSystem) -> LlvmBackend<'ctx, 'types> {
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
                Some(CType::Pointer { .. }) => self.context.i8_type().ptr_type(AddressSpace::default()).as_basic_type_enum(),
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
                        self.context.struct_type(&field_types, false).as_basic_type_enum()
                    }
                }
                Some(CType::Enum { .. }) => self.context.i32_type().as_basic_type_enum(),
                Some(CType::Function { .. }) => self.context.i32_type().as_basic_type_enum(),
                Some(CType::Typedef { underlying, .. }) => self.to_llvm_type(underlying.0),
                Some(CType::Qualified { base, .. }) => self.to_llvm_type(base.0),
                Some(CType::Union { .. }) => self.context.i8_type().array_type(1).as_basic_type_enum(),
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

    fn lower_translation_unit(&mut self, arena: &Arena, offset: NodeOffset) -> Result<(), BackendError> {
        eprintln!("lower_translation_unit: starting at offset={:?}", offset);
        let mut child_offset = offset;
        while child_offset != NodeOffset::NULL {
            if let Some(node) = arena.get(child_offset) {
                eprintln!("  visiting node kind={} data={} first_child={:?} next_sibling={:?}",
                    node.kind, node.data, node.first_child, node.next_sibling);
                match node.kind {
                    1..=9 | 83 => { eprintln!("    -> skipped type node"); }
                    20 => { eprintln!("    -> processing as decl"); self.lower_decl(arena, node)?; }
                    22 => { eprintln!("    -> processing as func_decl"); self.lower_func_decl(arena, node)?; }
                    23 => { eprintln!("    -> processing as func_def"); self.lower_func_def(arena, node)?; }
                    101..=105 => { eprintln!("    -> skipped storage class"); }
                    _ => { eprintln!("  WARNING: skipping unknown node kind={} at top level", node.kind); }
                }
                child_offset = node.next_sibling;
            } else {
                eprintln!("  child_offset {:?} is NULL/invalid, breaking", child_offset);
                break;
            }
        }
        eprintln!("lower_translation_unit: done");
        Ok(())
    }

    fn lower_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        eprintln!("  lower_decl: node kind={} first_child={:?}", node.kind, node.first_child);
        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                eprintln!("    decl child: kind={} data={} first_child={:?}", child.kind, child.data, child.first_child);
                match child.kind {
                    21 => { eprintln!("      -> var_decl"); self.lower_var_decl(arena, &child)?; }
                    22 => { eprintln!("      -> func_decl"); self.lower_func_decl(arena, &child)?; }
                    23 => { eprintln!("      -> func_def"); self.lower_func_def(arena, &child)?; }
                    _ => { eprintln!("      -> unknown, skipped"); }
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }
        Ok(())
    }

    fn lower_var_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let name = self.find_ident_name(arena, node);
        let var_name = name.as_deref().unwrap_or("var");
        let type_id = self.default_type();
        let alloca_type = self.to_llvm_type(type_id);
        let var_ptr = self.builder.build_alloca(alloca_type, var_name)
            .map_err(|_| BackendError::InvalidNode)?;
        if let Some(n) = name {
            self.variables.insert(n, var_ptr);
        }

        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                if matches!(child.kind, 64..=73 | 80..=82 | 60..=62) {
                    if let Some(val) = self.lower_expr(arena, child_offset)? {
                        let _ = self.builder.build_store(var_ptr, val)
                            .map_err(|_| BackendError::InvalidNode);
                    }
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }
        Ok(())
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
        let mut body_offset = NodeOffset::NULL;
        let mut return_type_id: u32 = 7;

        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                match child.kind {
                    2..=9 => {
                        if child.kind == 9 {
                            if let Some(ident) = arena.get(child.first_child) {
                                if ident.kind == 60 {
                                    if let Some(name) = arena.get_string(NodeOffset(ident.data)) {
                                        func_name = name.to_string();
                                    }
                                }
                                let mut sibling = ident.next_sibling;
                                while sibling != NodeOffset::NULL {
                                    if let Some(sib) = arena.get(sibling) {
                                        if sib.kind == 24 {
                                            let param_type_id = self.default_type();
                                            let bt = self.to_llvm_type(param_type_id);
                                            param_types.push(bt.into());
                                            let mut decl = sib.next_sibling;
                                            let mut pname = None;
                                            while decl != NodeOffset::NULL {
                                                if let Some(d) = arena.get(decl) {
                                                    if d.kind == 60 {
                                                        pname = arena.get_string(NodeOffset(d.data));
                                                        break;
                                                    }
                                                    decl = d.next_sibling;
                                                } else {
                                                    break;
                                                }
                                            }
                                            param_names.push(pname.unwrap_or("p").to_string());
                                        }
                                        sibling = sib.next_sibling;
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    2..=6 | 83 => {
                        if matches!(child.kind, 2..=6 | 83) {
                            return_type_id = match child.kind {
                                1 => 0,
                                2 => 2,
                                3 => 5,
                                4 => 9,
                                5 => 13,
                                6 => 14,
                                83 => 7,
                                _ => 7,
                            };
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

        let is_void_ret = self.is_void_type_id(return_type_id);
        let fn_type = if is_void_ret {
            self.context.void_type().fn_type(&param_types, false)
        } else {
            let ret_bt = self.to_llvm_type(return_type_id);
            ret_bt.fn_type(&param_types, false)
        };
        let function = self.module.add_function(&func_name, fn_type, None);
        self.functions.insert(func_name.clone(), function);

        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);

        self.variables.clear();

        for (i, pname) in param_names.iter().enumerate() {
            let param_type_id = self.default_type();
            let param_llvm_type = self.to_llvm_type(param_type_id);
            let param_ptr = self.builder.build_alloca(param_llvm_type, pname)
                .map_err(|_| BackendError::InvalidNode)?;
            let param_val = function.get_nth_param(i as u32)
                .ok_or(BackendError::InvalidNode)?;
            self.builder.build_store(param_ptr, param_val)
                .map_err(|_| BackendError::InvalidNode)?;
            self.variables.insert(pname.clone(), param_ptr);
        }

        if body_offset != NodeOffset::NULL {
            if let Some(body) = arena.get(body_offset) {
                self.lower_compound(arena, &body)?;
            }
        }

        let last_block = self.builder.get_insert_block();
        if let Some(bb) = last_block {
            if bb.get_terminator().is_none() {
                if is_void_ret {
                    self.builder.build_return(None)
                        .map_err(|_| BackendError::InvalidNode)?;
                } else {
                    let ret_bt = self.to_llvm_type(return_type_id);
                    let int_type = ret_bt.into_int_type();
                    self.builder.build_return(Some(&int_type.const_int(0, false)))
                        .map_err(|_| BackendError::InvalidNode)?;
                }
            }
        }

        Ok(())
    }

    fn lower_compound(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        eprintln!("lower_compound: first_child={:?}", node.first_child);
        let mut child_offset = node.first_child;
        let mut count = 0;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                eprintln!("  compound child {}: kind={}", count, child.kind);
                self.lower_stmt(arena, child_offset)?;
                child_offset = child.next_sibling;
                count += 1;
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

    fn lower_if_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let cond_offset = node.first_child;
        let then_offset = if let Some(c) = arena.get(cond_offset) { c.next_sibling } else { NodeOffset::NULL };
        let else_offset = if let Some(c) = arena.get(then_offset) { c.next_sibling } else { NodeOffset::NULL };

        let function = self.builder.get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or(BackendError::InvalidNode)?;

        let cond_val = self.lower_expr(arena, cond_offset)?
            .ok_or(BackendError::InvalidNode)?;

        let cond_int = if cond_val.is_int_value() {
            cond_val.into_int_value()
        } else {
            return Ok(());
        };

        let then_bb = self.context.append_basic_block(function, "then");
        let else_bb = self.context.append_basic_block(function, "else");
        let merge_bb = self.context.append_basic_block(function, "merge");

        self.builder.build_conditional_branch(cond_int, then_bb, else_bb)
            .map_err(|_| BackendError::InvalidNode)?;

        self.builder.position_at_end(then_bb);
        self.lower_stmt(arena, then_offset)?;
        if self.builder.get_insert_block().and_then(|bb| bb.get_terminator()).is_none() {
            self.builder.build_unconditional_branch(merge_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        self.builder.position_at_end(else_bb);
        if else_offset != NodeOffset::NULL {
            self.lower_stmt(arena, else_offset)?;
        }
        if self.builder.get_insert_block().and_then(|bb| bb.get_terminator()).is_none() {
            self.builder.build_unconditional_branch(merge_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        self.builder.position_at_end(merge_bb);
        Ok(())
    }

    fn lower_while_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let cond_offset = node.first_child;
        let body_offset = if let Some(c) = arena.get(cond_offset) { c.next_sibling } else { NodeOffset::NULL };

        let function = self.builder.get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or(BackendError::InvalidNode)?;

        let cond_bb = self.context.append_basic_block(function, "while.cond");
        let body_bb = self.context.append_basic_block(function, "while.body");
        let end_bb = self.context.append_basic_block(function, "while.end");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|_| BackendError::InvalidNode)?;

        self.builder.position_at_end(cond_bb);
        let cond_val = self.lower_expr(arena, cond_offset)?
            .ok_or(BackendError::InvalidNode)?;
        let cond_int = if cond_val.is_int_value() { cond_val.into_int_value() } else { return Ok(()); };
        self.builder.build_conditional_branch(cond_int, body_bb, end_bb)
            .map_err(|_| BackendError::InvalidNode)?;

        self.builder.position_at_end(body_bb);
        self.lower_stmt(arena, body_offset)?;
        if self.builder.get_insert_block().and_then(|bb| bb.get_terminator()).is_none() {
            self.builder.build_unconditional_branch(cond_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        self.builder.position_at_end(end_bb);
        Ok(())
    }

    fn lower_for_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let function = self.builder.get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or(BackendError::InvalidNode)?;

        let init_offset = node.first_child;
        if init_offset != NodeOffset::NULL {
            self.lower_stmt(arena, init_offset)?;
        }

        let cond_bb = self.context.append_basic_block(function, "for.cond");
        let body_bb = self.context.append_basic_block(function, "for.body");
        let end_bb = self.context.append_basic_block(function, "for.end");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|_| BackendError::InvalidNode)?;

        self.builder.position_at_end(cond_bb);
        let cond_offset = if let Some(c) = arena.get(init_offset) { c.next_sibling } else { NodeOffset::NULL };
        if cond_offset != NodeOffset::NULL {
            if let Some(cond_val) = self.lower_expr(arena, cond_offset)? {
                let cond_int = if cond_val.is_int_value() { cond_val.into_int_value() } else { return Ok(()); };
                self.builder.build_conditional_branch(cond_int, body_bb, end_bb)
                    .map_err(|_| BackendError::InvalidNode)?;
            } else {
                self.builder.build_unconditional_branch(body_bb)
                    .map_err(|_| BackendError::InvalidNode)?;
            }
        } else {
            self.builder.build_unconditional_branch(body_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        self.builder.position_at_end(body_bb);
        let body_offset = if let Some(c) = arena.get(cond_offset) { c.next_sibling } else { NodeOffset::NULL };
        if body_offset != NodeOffset::NULL {
            self.lower_stmt(arena, body_offset)?;
        }

        let incr_offset = if body_offset != NodeOffset::NULL {
            if let Some(c) = arena.get(body_offset) { c.next_sibling } else { NodeOffset::NULL }
        } else { NodeOffset::NULL };
        if incr_offset != NodeOffset::NULL {
            let _ = self.lower_expr(arena, incr_offset)?;
        }

        if self.builder.get_insert_block().and_then(|bb| bb.get_terminator()).is_none() {
            self.builder.build_unconditional_branch(cond_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        self.builder.position_at_end(end_bb);
        Ok(())
    }

    fn lower_return_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        eprintln!("lower_return_stmt: first_child={:?}", node.first_child);
        if node.first_child != NodeOffset::NULL {
            if let Some(val) = self.lower_expr(arena, node.first_child)? {
                eprintln!("lower_return_stmt: val is_int={} is_float={}", val.is_int_value(), val.is_float_value());
                if val.is_int_value() {
                    let int_val = val.into_int_value();
                    self.builder.build_return(Some(&int_val))
                        .map_err(|e| { eprintln!("build_return error: {:?}", e); BackendError::InvalidNode })?;
                } else if val.is_float_value() {
                    let float_val = val.into_float_value();
                    self.builder.build_return(Some(&float_val))
                        .map_err(|e| { eprintln!("build_return error: {:?}", e); BackendError::InvalidNode })?;
                } else {
                    eprintln!("lower_return_stmt: val not int/float, returning Ok");
                    return Ok(());
                }
            } else {
                eprintln!("lower_return_stmt: val is None, returning 0");
                self.builder.build_return(Some(&self.context.i32_type().const_int(0, false)))
                    .map_err(|_| BackendError::InvalidNode)?;
            }
        } else {
            eprintln!("lower_return_stmt: no first_child, returning void");
            self.builder.build_return(None)
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

    fn lower_expr(&mut self, arena: &Arena, offset: NodeOffset) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let node = match arena.get(offset) {
            Some(n) => n,
            None => return Ok(None),
        };

        if !node.flags.contains(NodeFlags::IS_VALID) {
            return Ok(None);
        }

        eprintln!("lower_expr: offset={:?} kind={} data={}", offset, node.kind, node.data);
        match node.kind {
            60 => self.lower_ident(arena, &node),
            61 => self.lower_int_const(&node),
            62 => self.lower_char_const(&node),
            63 | 81 => self.lower_string_const(&node),
            64 => self.lower_binop(arena, &node),
            65 => self.lower_unop(arena, &node),
            66 => self.lower_cond_expr(arena, &node),
            67 => self.lower_call_expr(arena, &node),
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

    fn lower_ident(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let name_offset = NodeOffset(node.data);
        if let Some(name) = arena.get_string(name_offset) {
            if let Some(ptr) = self.variables.get(name) {
                let val = self.builder.build_load(*ptr, name)
                    .map_err(|_| BackendError::InvalidNode)?;
                return Ok(Some(val));
            }
            if let Some(func) = self.functions.get(name) {
                return Ok(Some(func.as_global_value().as_basic_value_enum()));
            }
        }
        Ok(None)
    }

    fn lower_int_const(&self, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let value = node.data as u64;
        if let Some(ts) = self.types {
            let type_id = 7;
            let llvm_type = if let Some(t) = ts.get_type(TypeId(type_id)) {
                match t {
                    CType::Char { .. } => self.context.i8_type().const_int(value, false).into(),
                    CType::Short { .. } => self.context.i16_type().const_int(value, false).into(),
                    CType::Int { .. } => self.context.i32_type().const_int(value, false).into(),
                    CType::Long { .. } | CType::LongLong { .. } => self.context.i64_type().const_int(value, false).into(),
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

    fn lower_char_const(&self, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let value = node.data as u64;
        Ok(Some(self.context.i8_type().const_int(value, false).into()))
    }

    fn lower_string_const(&self, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let byte = node.data as u8;
        let string_val = self.context.const_string(&[byte], true);
        let global = self.module.add_global(string_val.get_type(), Some(AddressSpace::default()), "str");
        global.set_initializer(&string_val);
        global.set_constant(true);
        Ok(Some(global.as_pointer_value().into()))
    }

    fn lower_float_const(&self, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let val = node.data as f64;
        Ok(Some(self.context.f64_type().const_float(val).into()))
    }

    fn lower_binop(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let lhs_offset = node.first_child;
        let rhs_offset = node.next_sibling;

        eprintln!("lower_binop: node.data={} lhs_offset={:?} rhs_offset={:?}", node.data, lhs_offset, rhs_offset);
        let lhs_val = self.lower_expr(arena, lhs_offset)?
            .ok_or(BackendError::InvalidNode)?;
        let rhs_val = self.lower_expr(arena, rhs_offset)?
            .ok_or(BackendError::InvalidNode)?;

        eprintln!("lower_binop: lhs_val={} rhs_val={}", lhs_val, rhs_val);

        let use_float = lhs_val.is_float_value() || rhs_val.is_float_value();

        let result: BasicValueEnum = if use_float {
            let lhs_float = if lhs_val.is_float_value() {
                lhs_val.into_float_value()
            } else {
                let int_val = lhs_val.into_int_value();
                self.builder.build_unsigned_int_to_float(int_val, self.context.f64_type(), "uitofp")
                    .map_err(|_| BackendError::InvalidNode)?
            };
            let rhs_float = if rhs_val.is_float_value() {
                rhs_val.into_float_value()
            } else {
                let int_val = rhs_val.into_int_value();
                self.builder.build_unsigned_int_to_float(int_val, self.context.f64_type(), "uitofp")
                    .map_err(|_| BackendError::InvalidNode)?
            };

            match node.data {
                1 => self.builder.build_float_add(lhs_float, rhs_float, "fadd")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                2 => self.builder.build_float_sub(lhs_float, rhs_float, "fsub")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                3 => self.builder.build_float_mul(lhs_float, rhs_float, "fmul")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                4 => self.builder.build_float_div(lhs_float, rhs_float, "fdiv")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                6 => self.builder.build_float_compare(inkwell::FloatPredicate::OEQ, lhs_float, rhs_float, "feq")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                7 => self.builder.build_float_compare(inkwell::FloatPredicate::ONE, lhs_float, rhs_float, "fne")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                8 => self.builder.build_float_compare(inkwell::FloatPredicate::OLT, lhs_float, rhs_float, "flt")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                9 => self.builder.build_float_compare(inkwell::FloatPredicate::OGT, lhs_float, rhs_float, "fgt")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                10 => self.builder.build_float_compare(inkwell::FloatPredicate::OLE, lhs_float, rhs_float, "fle")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                11 => self.builder.build_float_compare(inkwell::FloatPredicate::OGE, lhs_float, rhs_float, "fge")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                _ => return Err(BackendError::InvalidOperator(node.data)),
            }
        } else {
            let lhs_int = lhs_val.into_int_value();
            let rhs_int = rhs_val.into_int_value();

            match node.data {
                1 => self.builder.build_int_add(lhs_int, rhs_int, "add")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                2 => self.builder.build_int_sub(lhs_int, rhs_int, "sub")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                3 => self.builder.build_int_mul(lhs_int, rhs_int, "mul")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                4 => self.builder.build_int_signed_div(lhs_int, rhs_int, "div")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                5 => self.builder.build_int_signed_rem(lhs_int, rhs_int, "rem")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                6 => self.builder.build_int_compare(inkwell::IntPredicate::EQ, lhs_int, rhs_int, "eq")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                7 => self.builder.build_int_compare(inkwell::IntPredicate::NE, lhs_int, rhs_int, "ne")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                8 => self.builder.build_int_compare(inkwell::IntPredicate::SLT, lhs_int, rhs_int, "lt")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                9 => self.builder.build_int_compare(inkwell::IntPredicate::SGT, lhs_int, rhs_int, "gt")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                10 => self.builder.build_int_compare(inkwell::IntPredicate::SLE, lhs_int, rhs_int, "le")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                11 => self.builder.build_int_compare(inkwell::IntPredicate::SGE, lhs_int, rhs_int, "ge")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                12 => self.builder.build_and(lhs_int, rhs_int, "and")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                13 => self.builder.build_or(lhs_int, rhs_int, "or")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                14 => self.builder.build_and(lhs_int, rhs_int, "bitand")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                15 => self.builder.build_or(lhs_int, rhs_int, "bitor")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                16 => self.builder.build_xor(lhs_int, rhs_int, "xor")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                17 => self.builder.build_left_shift(lhs_int, rhs_int, "shl")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                18 => self.builder.build_right_shift(lhs_int, rhs_int, false, "shr")
                    .map_err(|_| BackendError::InvalidNode)?.into(),
                19 => {
                    if lhs_val.is_pointer_value() {
                        self.builder.build_store(lhs_val.into_pointer_value(), rhs_int)
                            .map_err(|_| BackendError::InvalidNode)?;
                        rhs_int.into()
                    } else {
                        rhs_int.into()
                    }
                },
                _ => return Err(BackendError::InvalidOperator(node.data)),
            }
        };

        Ok(Some(result))
    }

    fn lower_unop(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let child_offset = node.first_child;
        let operand = self.lower_expr(arena, child_offset)?
            .ok_or(BackendError::InvalidNode)?;

        let result: BasicValueEnum = match node.data {
            1 => {
                if operand.is_float_value() {
                    self.builder.build_float_neg(operand.into_float_value(), "fneg")
                        .map_err(|_| BackendError::InvalidNode)?.into()
                } else {
                    let int_op = operand.into_int_value();
                    self.builder.build_int_neg(int_op, "neg")
                        .map_err(|_| BackendError::InvalidNode)?.into()
                }
            }
            2 => {
                if operand.is_int_value() {
                    let int_op = operand.into_int_value();
                    let zero = self.context.i32_type().const_int(0, false);
                    self.builder.build_int_compare(inkwell::IntPredicate::EQ, int_op, zero, "lnot")
                        .map_err(|_| BackendError::InvalidNode)?.into()
                } else {
                    operand
                }
            }
            3 => {
                if operand.is_int_value() {
                    let int_op = operand.into_int_value();
                    self.builder.build_not(int_op, "bnot")
                        .map_err(|_| BackendError::InvalidNode)?.into()
                } else {
                    operand
                }
            }
            4 => {
                operand
            }
            5 => {
                if operand.is_pointer_value() {
                    self.builder.build_load(operand.into_pointer_value(), "deref")
                        .map_err(|_| BackendError::InvalidNode)?.into()
                } else {
                    operand
                }
            }
            _ => return Err(BackendError::InvalidOperator(node.data)),
        };

        Ok(Some(result))
    }

    fn lower_cond_expr(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let cond_offset = node.first_child;
        let then_offset = if let Some(c) = arena.get(cond_offset) { c.next_sibling } else { NodeOffset::NULL };
        let else_offset = if let Some(c) = arena.get(then_offset) { c.next_sibling } else { NodeOffset::NULL };

        let cond_val = self.lower_expr(arena, cond_offset)?;
        let then_val = self.lower_expr(arena, then_offset)?;
        let else_val = self.lower_expr(arena, else_offset)?;

        let _ = else_val;
        Ok(then_val)
    }

    fn lower_call_expr(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let first_child_offset = node.first_child;
        let first_child = arena.get(first_child_offset);

        let func_name = first_child
            .and_then(|c| arena.get_string(NodeOffset(c.data)));

        eprintln!("    lower_call_expr: func_name={:?}", func_name);
        eprintln!("    available functions: {:?}", self.functions.keys().collect::<Vec<_>>());

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
                eprintln!("    found function in map, building call");
                let call_site = self.builder.build_call(*func, &args, "call")
                    .map_err(|_| BackendError::InvalidNode)?;
                return Ok(Some(match call_site.try_as_basic_value() {
                    inkwell::values::ValueKind::Basic(v) => v,
                    inkwell::values::ValueKind::Instruction(_) => self.context.i32_type().const_int(0, false).into(),
                }));
            }

            eprintln!("    function not found, trying external declaration");
            let fn_type = self.context.i32_type().fn_type(&[], true);
            let ext_func = self.module.add_function(name, fn_type, None);
            self.functions.insert(name.to_string(), ext_func);
            let call_site = self.builder.build_call(ext_func, &args, "call")
                .map_err(|_| BackendError::InvalidNode)?;
            return Ok(Some(match call_site.try_as_basic_value() {
                inkwell::values::ValueKind::Basic(v) => v,
                inkwell::values::ValueKind::Instruction(_) => self.context.i32_type().const_int(0, false).into(),
            }));
        }

        Ok(None)
    }

    fn lower_cast_expr(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
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

    fn lower_sizeof_expr(&self, _node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(Some(self.context.i64_type().const_int(std::mem::size_of::<i32>() as u64, false).into()))
    }

    fn lower_comma_expr(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
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

    fn lower_assign_expr(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let lhs_offset = node.first_child;
        let rhs_offset = node.next_sibling;

        let rhs_val = self.lower_expr(arena, rhs_offset)?
            .ok_or(BackendError::InvalidNode)?;

        let lhs_node = arena.get(lhs_offset).ok_or(BackendError::InvalidNode)?;
        if lhs_node.kind == 60 {
            let name_offset = NodeOffset(lhs_node.data);
            if let Some(name) = arena.get_string(name_offset) {
                if let Some(ptr) = self.variables.get(name) {
                    self.builder.build_store(*ptr, rhs_val)
                        .map_err(|_| BackendError::InvalidNode)?;
                }
            }
        }

        Ok(Some(rhs_val))
    }

    fn lower_typeof(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        if node.first_child != NodeOffset::NULL {
            self.lower_expr(arena, node.first_child)
        } else {
            Ok(None)
        }
    }

    fn lower_stmt_expr(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
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

    fn lower_label_addr(&mut self, _arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let ptr = self.context.i8_type().ptr_type(AddressSpace::default()).const_null();
        Ok(Some(ptr.into()))
    }

    fn lower_builtin_call(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let builtin_name = if node.data != 0 {
            arena.get_string(NodeOffset(node.data)).map(|s| s.to_string())
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
                        inkwell::values::BasicMetadataValueEnum::IntValue(v) => return Ok(Some((*v).into())),
                        inkwell::values::BasicMetadataValueEnum::PointerValue(v) => return Ok(Some((*v).into())),
                        inkwell::values::BasicMetadataValueEnum::FloatValue(v) => return Ok(Some((*v).into())),
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
            "__builtin_offsetof" => {
                Ok(Some(self.context.i64_type().const_int(0, false).into()))
            }
            "__builtin_memcpy" | "__builtin_memset" | "__builtin_strlen" => {
                let intrinsic_name = match builtin_name.as_str() {
                    "__builtin_memcpy" => "llvm.memcpy.p0.p0.i64",
                    "__builtin_memset" => "llvm.memset.p0.i64",
                    "__builtin_strlen" => "llvm.strlen",
                    _ => unreachable!(),
                };

                if let Some(func) = self.functions.get(&builtin_name) {
                    let call_site = self.builder.build_call(*func, &args, "builtin_call")
                        .map_err(|_| BackendError::InvalidNode)?;
                    return Ok(Some(match call_site.try_as_basic_value() {
                        inkwell::values::ValueKind::Basic(v) => v,
                        _ => self.context.i32_type().const_int(0, false).into(),
                    }));
                }

                let fn_type = self.context.i64_type().fn_type(&args.iter().map(|a| {
                    match a {
                        inkwell::values::BasicMetadataValueEnum::IntValue(v) => v.get_type().into(),
                        inkwell::values::BasicMetadataValueEnum::PointerValue(v) => v.get_type().into(),
                        inkwell::values::BasicMetadataValueEnum::FloatValue(v) => v.get_type().into(),
                        _ => self.context.i64_type().into(),
                    }
                }).collect::<Vec<_>>(), false);
                let func = self.module.add_function(&builtin_name, fn_type, None);
                self.functions.insert(builtin_name.clone(), func);
                let call_site = self.builder.build_call(func, &args, "builtin_call")
                    .map_err(|_| BackendError::InvalidNode)?;
                Ok(Some(match call_site.try_as_basic_value() {
                    inkwell::values::ValueKind::Basic(v) => v,
                    _ => self.context.i32_type().const_int(0, false).into(),
                }))
            }
            "__builtin_va_arg" => {
                Ok(Some(self.context.i32_type().const_int(0, false).into()))
            }
            "__builtin_types_compatible_p" => {
                Ok(Some(self.context.i32_type().const_int(1, false).into()))
            }
            "__builtin_choose_expr" => {
                if args.len() >= 3 {
                    match &args[1] {
                        inkwell::values::BasicMetadataValueEnum::IntValue(v) => return Ok(Some((*v).into())),
                        inkwell::values::BasicMetadataValueEnum::PointerValue(v) => return Ok(Some((*v).into())),
                        inkwell::values::BasicMetadataValueEnum::FloatValue(v) => return Ok(Some((*v).into())),
                        _ => return Ok(None),
                    }
                }
                Ok(None)
            }
            _ => {
                if let Some(func) = self.functions.get(&builtin_name) {
                    let call_site = self.builder.build_call(*func, &args, "builtin_call")
                        .map_err(|_| BackendError::InvalidNode)?;
                    return Ok(Some(match call_site.try_as_basic_value() {
                        inkwell::values::ValueKind::Basic(v) => v,
                        _ => self.context.i32_type().const_int(0, false).into(),
                    }));
                }

                let fn_type = self.context.i32_type().fn_type(&[], true);
                let func = self.module.add_function(&builtin_name, fn_type, None);
                self.functions.insert(builtin_name.clone(), func);
                let call_site = self.builder.build_call(func, &args, "builtin_call")
                    .map_err(|_| BackendError::InvalidNode)?;
                Ok(Some(match call_site.try_as_basic_value() {
                    inkwell::values::ValueKind::Basic(v) => v,
                    _ => self.context.i32_type().const_int(0, false).into(),
                }))
            }
        }
    }

    fn lower_designated_init(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
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

    fn lower_extension(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
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
                    return arena.get_string(NodeOffset(child.data)).map(|s| s.to_string());
                }
                if matches!(child.kind, 7..=9) {
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

    pub fn dump_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    pub fn verify(&self) -> Result<(), BackendError> {
        if self.module.verify().is_err() {
            return Err(BackendError::VerificationFailed(self.module.print_to_string().to_string()));
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

pub fn create_backend<'ctx>(context: &'ctx Context, module_name: &str) -> LlvmBackend<'ctx, 'static> {
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

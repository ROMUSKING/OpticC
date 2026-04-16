use crate::arena::{Arena, CAstNode, NodeOffset, NodeFlags};
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::builder::Builder;
use inkwell::types::{BasicType, BasicTypeEnum, BasicMetadataTypeEnum};
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue, IntValue, BasicValue};
use inkwell::AddressSpace;
use std::collections::HashMap;

pub struct LlvmBackend<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    variables: HashMap<String, PointerValue<'ctx>>,
    functions: HashMap<String, FunctionValue<'ctx>>,
    vectorization_hints: VectorizationHints,
}

#[derive(Default)]
pub struct VectorizationHints {
    pub loop_vectorize: bool,
    pub vector_width: u32,
}

impl<'ctx> LlvmBackend<'ctx> {
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
        }
    }

    pub fn set_vectorization_hints(&mut self, hints: VectorizationHints) {
        self.vectorization_hints = hints;
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
                    // Skip type nodes at top level
                    1..=9 | 83 => { eprintln!("    -> skipped type node"); }
                    // Process declarations and function definitions
                    20 => { eprintln!("    -> processing as decl"); self.lower_decl(arena, node)?; }
                    22 => { eprintln!("    -> processing as func_decl"); self.lower_func_decl(arena, node)?; }
                    23 => { eprintln!("    -> processing as func_def"); self.lower_func_def(arena, node)?; }
                    // Skip storage class specifiers
                    101..=105 => { eprintln!("    -> skipped storage class"); }
                    // Skip other nodes we can't handle at top level
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
        // Find the identifier child
        let name = self.find_ident_name(arena, node);
        let var_name = name.as_deref().unwrap_or("var");
        let var_ptr = self.builder.build_alloca(self.context.i32_type(), var_name)
            .map_err(|_| BackendError::InvalidNode)?;
        if let Some(n) = name {
            self.variables.insert(n, var_ptr);
        }

        // Check for initializer
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

        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                match child.kind {
                    2..=9 => {
                        if child.kind == 9 { // FUNC type - contains identifier and params
                            if let Some(ident) = arena.get(child.first_child) {
                                if ident.kind == 60 { // IDENT
                                    if let Some(name) = arena.get_string(NodeOffset(ident.data)) {
                                        func_name = name;
                                    }
                                }
                                // Walk siblings of IDENT to find PARAM nodes
                                let mut sibling = ident.next_sibling;
                                while sibling != NodeOffset::NULL {
                                    if let Some(sib) = arena.get(sibling) {
                                        if sib.kind == 24 { // PARAM
                                            param_types.push(self.context.i32_type().into());
                                            // Walk PARAM's next_sibling to find the IDENT (declarator)
                                            let mut decl = sib.next_sibling;
                                            let mut pname = None;
                                            while decl != NodeOffset::NULL {
                                                if let Some(d) = arena.get(decl) {
                                                    if d.kind == 60 { // IDENT
                                                        pname = arena.get_string(NodeOffset(d.data));
                                                        break;
                                                    }
                                                    decl = d.next_sibling;
                                                } else {
                                                    break;
                                                }
                                            }
                                            param_names.push(pname.unwrap_or_else(|| "p".to_string()));
                                        }
                                        sibling = sib.next_sibling;
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    40 => { // COMPOUND - function body
                        body_offset = child_offset;
                    }
                    _ => {}
                }
                child_offset = child.next_sibling;
            } else {
                break;
            }
        }

        let fn_type = self.context.i32_type().fn_type(&param_types, false);
        let function = self.module.add_function(&func_name, fn_type, None);
        self.functions.insert(func_name.clone(), function);

        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);

        // Clear local variables for new function scope
        self.variables.clear();

        // Allocate parameters
        for (i, pname) in param_names.iter().enumerate() {
            let param_ptr = self.builder.build_alloca(self.context.i32_type(), pname)
                .map_err(|_| BackendError::InvalidNode)?;
            let param_val = function.get_nth_param(i as u32)
                .ok_or(BackendError::InvalidNode)?;
            self.builder.build_store(param_ptr, param_val)
                .map_err(|_| BackendError::InvalidNode)?;
            self.variables.insert(pname.clone(), param_ptr);
        }

        // Lower function body
        if body_offset != NodeOffset::NULL {
            if let Some(body) = arena.get(body_offset) {
                self.lower_compound(arena, &body)?;
            }
        }

        // Add implicit return 0 if no terminator
        let last_block = self.builder.get_insert_block();
        if let Some(bb) = last_block {
            if bb.get_terminator().is_none() {
                self.builder.build_return(Some(&self.context.i32_type().const_int(0, false)))
                    .map_err(|_| BackendError::InvalidNode)?;
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
            20 => self.lower_decl(arena, &node),  // AST_DECL
            21 => self.lower_var_decl(arena, &node),  // AST_VAR_DECL
            22 => self.lower_func_decl(arena, &node),  // AST_FUNC_DECL
            23 => self.lower_func_def(arena, &node),  // AST_FUNC_DEF
            40 => self.lower_compound(arena, &node),  // AST_COMPOUND
            41 => self.lower_if_stmt(arena, &node),  // AST_IF
            42 => self.lower_while_stmt(arena, &node),  // AST_WHILE
            43 => self.lower_for_stmt(arena, &node),  // AST_FOR
            44 => self.lower_return_stmt(arena, &node),  // AST_RETURN
            45 => self.lower_expr_stmt(arena, &node),  // AST_EXPR_STMT
            48 => Ok(()),  // AST_EMPTY
            46 | 47 => Ok(()),  // AST_BREAK / AST_CONTINUE - TODO
            50 => Ok(()),  // AST_SWITCH - TODO
            // Skip type/specifier nodes
            1..=9 | 83 | 90..=94 | 101..=105 => Ok(()),
            24 => Ok(()),  // AST_PARAM - handled in func_def
            25 | 26 => Ok(()),  // AST_STRUCT_DECL / AST_ENUM_CONST
            _ => {
                // Try as expression
                let _ = self.lower_expr(arena, offset)?;
                Ok(())
            }
        }
    }

    fn lower_if_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        let cond_offset = node.first_child;
        let then_offset = if let Some(c) = arena.get(cond_offset) { c.next_sibling } else { NodeOffset::NULL };
        let else_offset = if let Some(c) = arena.get(then_offset) { c.next_sibling } else { NodeOffset::NULL };

        // Get the current function
        let function = self.builder.get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or(BackendError::InvalidNode)?;

        // Lower condition
        let cond_val = self.lower_expr(arena, cond_offset)?
            .ok_or(BackendError::InvalidNode)?;

        let cond_int = if cond_val.is_int_value() {
            cond_val.into_int_value()
        } else {
            return Ok(()); // Can't handle non-int conditions yet
        };

        let then_bb = self.context.append_basic_block(function, "then");
        let else_bb = self.context.append_basic_block(function, "else");
        let merge_bb = self.context.append_basic_block(function, "merge");

        self.builder.build_conditional_branch(cond_int, then_bb, else_bb)
            .map_err(|_| BackendError::InvalidNode)?;

        // Then block
        self.builder.position_at_end(then_bb);
        self.lower_stmt(arena, then_offset)?;
        if self.builder.get_insert_block().and_then(|bb| bb.get_terminator()).is_none() {
            self.builder.build_unconditional_branch(merge_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        // Else block
        self.builder.position_at_end(else_bb);
        if else_offset != NodeOffset::NULL {
            self.lower_stmt(arena, else_offset)?;
        }
        if self.builder.get_insert_block().and_then(|bb| bb.get_terminator()).is_none() {
            self.builder.build_unconditional_branch(merge_bb)
                .map_err(|_| BackendError::InvalidNode)?;
        }

        // Merge block
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
        // For loop: init, cond, incr, body stored as children
        let function = self.builder.get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or(BackendError::InvalidNode)?;

        // Lower init
        let init_offset = node.first_child;
        if init_offset != NodeOffset::NULL {
            self.lower_stmt(arena, init_offset)?;
        }

        let cond_bb = self.context.append_basic_block(function, "for.cond");
        let body_bb = self.context.append_basic_block(function, "for.body");
        let end_bb = self.context.append_basic_block(function, "for.end");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|_| BackendError::InvalidNode)?;

        // Condition
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

        // Body
        self.builder.position_at_end(body_bb);
        let body_offset = if let Some(c) = arena.get(cond_offset) { c.next_sibling } else { NodeOffset::NULL };
        if body_offset != NodeOffset::NULL {
            self.lower_stmt(arena, body_offset)?;
        }

        // Increment
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
                eprintln!("lower_return_stmt: val is_int={}", val.is_int_value());
                let int_val = if val.is_int_value() {
                    val.into_int_value()
                } else {
                    eprintln!("lower_return_stmt: val not int, returning Ok");
                    return Ok(());
                };
                self.builder.build_return(Some(&int_val))
                    .map_err(|e| { eprintln!("build_return error: {:?}", e); BackendError::InvalidNode })?;
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
            60 => self.lower_ident(arena, &node),       // AST_IDENT
            61 => self.lower_int_const(&node),           // AST_INT_CONST
            62 => self.lower_char_const(&node),          // AST_CHAR_CONST
            63 | 81 => self.lower_string_const(&node),   // AST_STRING
            64 => self.lower_binop(arena, &node),        // AST_BINOP
            65 => self.lower_unop(arena, &node),         // AST_UNOP
            66 => self.lower_cond_expr(arena, &node),    // AST_COND (ternary)
            67 => self.lower_call_expr(arena, &node),    // AST_CALL
            70 => self.lower_cast_expr(arena, &node),    // AST_CAST
            71 => self.lower_sizeof_expr(&node),         // AST_SIZEOF
            72 => self.lower_comma_expr(arena, &node),   // AST_COMMA
            73 => self.lower_assign_expr(arena, &node),  // AST_ASSIGN
            80 => self.lower_int_const(&node),           // AST_NUM
            82 => self.lower_float_const(&node),         // AST_FLOAT_CONST
            // Skip type nodes that appear in expression context
            1..=9 | 83 | 90..=94 | 101..=105 => Ok(None),
            _ => Ok(None),
        }
    }

    fn lower_ident(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let name_offset = NodeOffset(node.data);
        if let Some(name) = arena.get_string(name_offset) {
            if let Some(ptr) = self.variables.get(&name) {
                let val = self.builder.build_load(*ptr, &name)
                    .map_err(|_| BackendError::InvalidNode)?;
                return Ok(Some(val));
            }
            if let Some(func) = self.functions.get(&name) {
                return Ok(Some(func.as_global_value().as_basic_value_enum()));
            }
        }
        Ok(None)
    }

    fn lower_int_const(&self, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let value = node.data as u64;
        Ok(Some(self.context.i32_type().const_int(value, false).into()))
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
        let rhs_offset = node.next_sibling;  // RHS is stored as next_sibling of BINOP node

        eprintln!("lower_binop: node.data={} lhs_offset={:?} rhs_offset={:?}", node.data, lhs_offset, rhs_offset);
        let lhs_val = self.lower_expr(arena, lhs_offset)?
            .ok_or(BackendError::InvalidNode)?;
        let rhs_val = self.lower_expr(arena, rhs_offset)?
            .ok_or(BackendError::InvalidNode)?;

        eprintln!("lower_binop: lhs_val={} rhs_val={}", lhs_val, rhs_val);
        let lhs_int = lhs_val.into_int_value();
        let rhs_int = rhs_val.into_int_value();

        let result: BasicValueEnum = match node.data {
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
                // Assignment - lhs should be a variable pointer
                if lhs_val.is_pointer_value() {
                    self.builder.build_store(lhs_val.into_pointer_value(), rhs_int)
                        .map_err(|_| BackendError::InvalidNode)?;
                    rhs_int.into()
                } else {
                    rhs_int.into()
                }
            },
            _ => return Err(BackendError::InvalidOperator(node.data)),
        };

        Ok(Some(result))
    }

    fn lower_unop(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let child_offset = node.first_child;
        let operand = self.lower_expr(arena, child_offset)?
            .ok_or(BackendError::InvalidNode)?;

        let result: BasicValueEnum = match node.data {
            1 => { // OP_NEG
                let int_op = operand.into_int_value();
                self.builder.build_int_neg(int_op, "neg")
                    .map_err(|_| BackendError::InvalidNode)?.into()
            }
            2 => { // OP_NOT (logical)
                let int_op = operand.into_int_value();
                let zero = self.context.i32_type().const_int(0, false);
                self.builder.build_int_compare(inkwell::IntPredicate::EQ, int_op, zero, "lnot")
                    .map_err(|_| BackendError::InvalidNode)?.into()
            }
            3 => { // OP_BITNOT
                let int_op = operand.into_int_value();
                self.builder.build_not(int_op, "bnot")
                    .map_err(|_| BackendError::InvalidNode)?.into()
            }
            4 => { // OP_ADDR - return the pointer as-is if it's a pointer
                operand
            }
            5 => { // OP_DEREF
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
        // Ternary: cond ? then : else
        let cond_offset = node.first_child;
        let then_offset = if let Some(c) = arena.get(cond_offset) { c.next_sibling } else { NodeOffset::NULL };
        let else_offset = if let Some(c) = arena.get(then_offset) { c.next_sibling } else { NodeOffset::NULL };

        let cond_val = self.lower_expr(arena, cond_offset)?;
        let then_val = self.lower_expr(arena, then_offset)?;
        let else_val = self.lower_expr(arena, else_offset)?;

        // Simple approach: just evaluate both sides and return then_val
        // Proper implementation would use phi nodes
        let _ = else_val;
        Ok(then_val)
    }

    fn lower_call_expr(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let first_child_offset = node.first_child;
        let first_child = arena.get(first_child_offset);

        // Try to find function name
        let func_name = first_child
            .and_then(|c| arena.get_string(NodeOffset(c.data)));

        eprintln!("    lower_call_expr: func_name={:?}", func_name);
        eprintln!("    available functions: {:?}", self.functions.keys().collect::<Vec<_>>());

        // Collect arguments
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

        if let Some(name) = &func_name {
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
            // Try to declare an external function
            let fn_type = self.context.i32_type().fn_type(&[], true);
            let ext_func = self.module.add_function(name, fn_type, None);
            self.functions.insert(name.clone(), ext_func);
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
        // Cast: just return the child expression value
        let child_offset = node.first_child;
        if child_offset != NodeOffset::NULL {
            // Skip type child, get expr child
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
        let rhs_offset = node.next_sibling;  // RHS is stored as next_sibling of ASSIGN node

        let rhs_val = self.lower_expr(arena, rhs_offset)?
            .ok_or(BackendError::InvalidNode)?;

        // Find the variable pointer for lhs
        let lhs_node = arena.get(lhs_offset).ok_or(BackendError::InvalidNode)?;
        if lhs_node.kind == 60 { // AST_IDENT
            let name_offset = NodeOffset(lhs_node.data);
            if let Some(name) = arena.get_string(name_offset) {
                if let Some(ptr) = self.variables.get(&name) {
                    let val_to_store = if rhs_val.is_int_value() {
                        rhs_val.into_int_value().into()
                    } else {
                        rhs_val
                    };
                    self.builder.build_store(*ptr, val_to_store)
                        .map_err(|_| BackendError::InvalidNode)?;
                }
            }
        }

        Ok(Some(rhs_val))
    }

    // Helper: find the first identifier name in a node's children
    fn find_ident_name(&self, arena: &Arena, node: &CAstNode) -> Option<String> {
        let mut child_offset = node.first_child;
        while child_offset != NodeOffset::NULL {
            if let Some(child) = arena.get(child_offset) {
                if child.kind == 60 { // AST_IDENT
                    return arena.get_string(NodeOffset(child.data));
                }
                // Recurse into declarator nodes
                if matches!(child.kind, 7..=9) { // PTR, ARRAY, FUNC types
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
        // Pass manager API changed in inkwell 0.9 - skip for now
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

pub fn create_backend<'ctx>(context: &'ctx Context, module_name: &str) -> LlvmBackend<'ctx> {
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
}

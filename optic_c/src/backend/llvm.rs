use crate::arena::{Arena, CAstNode, NodeOffset, NodeFlags};
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::builder::Builder;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{BasicValue, BasicValueEnum, FunctionValue, PointerValue};
use inkwell::AddressSpace;
use std::collections::HashMap;

pub struct LlvmBackend<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    strings: HashMap<NodeOffset, PointerValue<'ctx>>,
    variables: HashMap<NodeOffset, PointerValue<'ctx>>,
    functions: HashMap<NodeOffset, FunctionValue<'ctx>>,
    vectorization_hints: VectorizationHints,
}

#[derive(Default)]
pub struct VectorizationHints {
    pub loop_vectorize: bool,
    pub vector_width: u32,
    pub alignment_hints: HashMap<NodeOffset, u32>,
}

impl<'ctx> LlvmBackend<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        let module = context.create_module(module_name);
        let builder = context.create_builder();
        LlvmBackend {
            context,
            module,
            builder,
            strings: HashMap::new(),
            variables: HashMap::new(),
            functions: HashMap::new(),
            vectorization_hints: VectorizationHints::default(),
        }
    }

    pub fn set_vectorization_hints(&mut self, hints: VectorizationHints) {
        self.vectorization_hints = hints;
    }

    pub fn compile(&mut self, arena: &Arena, root: NodeOffset) -> Result<(), BackendError> {
        self.lower_nodes(arena, root)?;
        Ok(())
    }

    pub fn module(&self) -> &Module<'ctx> {
        &self.module
    }

    fn lower_nodes(&mut self, arena: &Arena, offset: NodeOffset) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let node = match arena.get(offset) {
            Some(n) => n,
            None => return Ok(None),
        };

        if !node.flags.contains(NodeFlags::IS_VALID) {
            return Ok(None);
        }

        // Only handle value-producing nodes, delegate type nodes to lower_type
        let result: Option<BasicValueEnum<'ctx>> = match node.kind {
            // Declarations - return None as they don't produce values
            20 | 21 | 22 | 23 | 24 | 25 | 26 => {
                self.lower_decl(arena, node)?;
                None
            }
            // Statements - return None as they don't produce values
            40 | 41 | 42 | 43 | 44 | 45 | 46 | 47 | 48 | 50 => {
                self.lower_stmt(arena, node)?;
                None
            }
            // Expressions - produce values
            60 | 61 | 62 | 63 | 64 | 65 | 66 | 67 | 68 | 69 | 70 | 71 | 72 | 73 => {
                self.lower_expr(arena, node)?
            }
            // Literals
            80 | 81 | 82 => {
                self.lower_literal(arena, node)?
            }
            // Skip type nodes and storage class specifiers (they don't produce values)
            1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 83 => None,
            // Storage class specifiers: typedef=101, extern=102, static=103, auto=104, register=105
            101 | 102 | 103 | 104 | 105 => None,
            _ => return Err(BackendError::UnknownNodeKind(node.kind)),
        };

        Ok(result)
    }

    fn lower_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<(), BackendError> {
        match node.kind {
            40 => { let _ = self.lower_compound(arena, node)?; }
            41 => { let _ = self.lower_if(arena, node)?; }
            42 => { let _ = self.lower_while(arena, node)?; }
            43 => { let _ = self.lower_for(arena, node)?; }
            44 => { let _ = self.lower_return(arena, node)?; }
            45 => { let _ = self.lower_expr_stmt(arena, node)?; }
            46 => { let _ = self.lower_break(); }
            47 => { let _ = self.lower_continue(); }
            50 => { let _ = self.lower_switch(arena, node)?; }
            _ => {}
        }
        Ok(())
    }

    fn lower_expr(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        match node.kind {
            60 => self.lower_ident(arena, node),
            61 => self.lower_int_const(arena, node),
            62 => self.lower_char_const(arena, node),
            63 => self.lower_string(arena, node),
            64 => self.lower_binop(arena, node),
            65 => self.lower_unop(arena, node),
            66 => self.lower_cond(arena, node),
            67 => self.lower_call(arena, node),
            68 => self.lower_array_subscript(arena, node),
            69 => self.lower_member(arena, node),
            70 => self.lower_cast(arena, node),
            71 => self.lower_sizeof(arena, node),
            72 => self.lower_comma(arena, node),
            73 => self.lower_assign(arena, node),
            _ => Err(BackendError::UnknownNodeKind(node.kind)),
        }
    }

    fn lower_literal(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        match node.kind {
            80 => self.lower_num(arena, node),
            81 => self.lower_string(arena, node),
            82 => self.lower_float_const(arena, node),
            _ => Err(BackendError::UnknownNodeKind(node.kind)),
        }
    }

    fn lower_type(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicTypeEnum<'ctx>>, BackendError> {
        let child_offset = node.first_child;
        let child = arena.get(child_offset);
        match node.kind {
            1 => Ok(Some(self.context.i32_type().into())),
            2 => Ok(Some(self.context.i32_type().into())),
            3 => Ok(Some(self.context.i8_type().into())),
            4 | 5 => self.lower_struct_union_type(arena, node),
            6 => Ok(Some(self.context.i32_type().into())),
            7 => self.lower_ptr_type(arena, node),
            8 => self.lower_array_type(arena, node),
            9 => self.lower_func_type(arena, node),
            83 => Ok(Some(self.context.f32_type().into())),
            _ => Err(BackendError::UnknownNodeKind(node.kind)),
        }
    }

    fn lower_struct_union_type(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicTypeEnum<'ctx>>, BackendError> {
        let mut struct_fields: Vec<BasicTypeEnum> = Vec::new();
        let mut child_offset = node.first_child;
        while child_offset.0 != 0 {
            let c = arena.get(child_offset).ok_or(BackendError::InvalidNode)?;
            if let Some(field_type) = self.lower_type(arena, &c)? {
                struct_fields.push(field_type);
            }
            child_offset = c.next_sibling;
        }
        let struct_type = self.context.struct_type(&struct_fields, false);
        Ok(Some(struct_type.into()))
    }

    fn lower_ptr_type(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicTypeEnum<'ctx>>, BackendError> {
        let child = arena.get(node.first_child).ok_or(BackendError::InvalidNode)?;
        let pointee_type = self.lower_type(arena, &child)?;
        match pointee_type {
            Some(t) => Ok(Some(t.ptr_type(AddressSpace::default()).into())),
            None => Ok(Some(self.context.i8_type().ptr_type(AddressSpace::default()).into())),
        }
    }

    fn lower_array_type(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicTypeEnum<'ctx>>, BackendError> {
        let child = arena.get(node.first_child).ok_or(BackendError::InvalidNode)?;
        let element_type = self.lower_type(arena, &child)?;
        let size = node.data;
        match element_type {
            Some(t) => Ok(Some(t.array_type(size).into())),
            None => Ok(Some(self.context.i8_type().array_type(size).into())),
        }
    }

    fn lower_func_type(&self, _arena: &Arena, _node: &CAstNode) -> Result<Option<BasicTypeEnum<'ctx>>, BackendError> {
        // Function types are handled via pointer types, return None here
        Ok(None)
    }

    fn lower_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let mut child_offset = node.first_child;
        while child_offset.0 != 0 {
            let c = arena.get(child_offset).ok_or(BackendError::InvalidNode)?;
            self.lower_nodes(arena, child_offset)?;
            child_offset = c.next_sibling;
        }
        Ok(None)
    }

    fn lower_var_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let name_offset = NodeOffset(node.data);
        let var_ptr = self.builder.build_alloca(self.context.i32_type(), "var")?;
        self.variables.insert(name_offset, var_ptr);
        Ok(None)
    }

    fn lower_func_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let name_offset = NodeOffset(node.data);
        let fn_type = self.context.i32_type().fn_type(&[], false);
        let function = self.module.add_function("func", fn_type, None);
        self.functions.insert(name_offset, function);
        Ok(None)
    }

    fn lower_func_def(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let name_offset = NodeOffset(node.data);
        let fn_type = self.context.i32_type().fn_type(&[], false);
        let function = self.module.add_function("func", fn_type, None);
        
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        
        self.functions.insert(name_offset, function);
        
        if self.vectorization_hints.loop_vectorize {
        }
        
        let mut child_offset = node.first_child;
        while child_offset.0 != 0 {
            let c = arena.get(child_offset).ok_or(BackendError::InvalidNode)?;
            self.lower_nodes(arena, child_offset)?;
            child_offset = c.next_sibling;
        }
        
        self.builder.build_return(Some(&self.context.i32_type().const_int(0, false)))?;
        Ok(None)
    }

    fn lower_param(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(None)
    }

    fn lower_struct_decl(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(None)
    }

    fn lower_enum_const(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(None)
    }

    fn lower_compound(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let mut last_value: Option<BasicValueEnum> = None;
        let mut child_offset = node.first_child;
        while child_offset.0 != 0 {
            let c = arena.get(child_offset).ok_or(BackendError::InvalidNode)?;
            last_value = self.lower_nodes(arena, child_offset)?;
            child_offset = c.next_sibling;
        }
        Ok(last_value)
    }

    fn lower_if(&mut self, _arena: &Arena, _node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        // Simplified - control flow lowering requires block management
        Ok(None)
    }

    fn lower_while(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(None)
    }

    fn lower_for(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(None)
    }

    fn lower_return(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let child_offset = node.first_child;
        match arena.get(child_offset) {
            Some(c) => {
                let value = self.lower_nodes(arena, child_offset)?;
                if let Some(v) = value {
                    self.builder.build_return(Some(&v))?;
                }
            }
            None => {
                self.builder.build_return(None)?;
            }
        }
        Ok(None)
    }

    fn lower_expr_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let child_offset = node.first_child;
        match arena.get(child_offset) {
            Some(_) => self.lower_nodes(arena, child_offset),
            None => Ok(None),
        }
    }

    fn lower_break(&mut self) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        self.builder.build_unconditional_branch(self.context.append_basic_block(*self.functions.values().next().unwrap(), ""))?;
        Ok(None)
    }

    fn lower_continue(&mut self) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(None)
    }

    fn lower_switch(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(None)
    }

    fn lower_ident(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let name_offset = NodeOffset(node.data);
        if let Some(ptr) = self.variables.get(&name_offset) {
            let value = self.builder.build_load(*ptr, "ident")?;
            return Ok(Some(value));
        }
        if let Some(func) = self.functions.get(&name_offset) {
            return Ok(Some(func.as_global_value().as_basic_value_enum()));
        }
        Err(BackendError::UndefinedVariable(name_offset))
    }

    fn lower_int_const(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let value = node.data as u64;
        Ok(Some(self.context.i32_type().const_int(value, false).into()))
    }

    fn lower_char_const(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let value = node.data as u64;
        Ok(Some(self.context.i8_type().const_int(value, false).into()))
    }

    fn lower_string(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let string_const = self.context.const_string(&[node.data as u8], true);
        let global = self.module.add_global(string_const.get_type(), Some(AddressSpace::default()), "str");
        global.set_initializer(&string_const);
        Ok(Some(global.as_basic_value_enum()))
    }

    fn lower_binop(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let lhs_offset = node.first_child;
        let lhs_opt = self.lower_nodes(arena, lhs_offset)?;
        let lhs_basic = lhs_opt.ok_or(BackendError::InvalidNode)?;
        let lhs_val = lhs_basic.into_int_value();
        
        let rhs_val = if let Some(c) = arena.get(lhs_offset) {
            let rhs_offset = c.next_sibling;
            let rhs_opt = self.lower_nodes(arena, rhs_offset)?;
            let rhs_basic = rhs_opt.ok_or(BackendError::InvalidNode)?;
            rhs_basic.into_int_value()
        } else {
            return Err(BackendError::InvalidNode);
        };
        
        let result: BasicValueEnum<'ctx> = match node.data {
            1 => self.builder.build_int_add(lhs_val, rhs_val, "addtmp")?.into(),
            2 => self.builder.build_int_sub(lhs_val, rhs_val, "subtmp")?.into(),
            3 => self.builder.build_int_mul(lhs_val, rhs_val, "multmp")?.into(),
            4 => self.builder.build_int_signed_div(lhs_val, rhs_val, "divtmp")?.into(),
            5 => self.builder.build_int_signed_rem(lhs_val, rhs_val, "modtmp")?.into(),
            6 => self.builder.build_int_compare(inkwell::IntPredicate::EQ, lhs_val, rhs_val, "eqtmp")?.into(),
            7 => self.builder.build_int_compare(inkwell::IntPredicate::NE, lhs_val, rhs_val, "netmp")?.into(),
            8 => self.builder.build_int_compare(inkwell::IntPredicate::SLT, lhs_val, rhs_val, "lttmp")?.into(),
            9 => self.builder.build_int_compare(inkwell::IntPredicate::SGT, lhs_val, rhs_val, "gttmp")?.into(),
            10 => self.builder.build_int_compare(inkwell::IntPredicate::SLE, lhs_val, rhs_val, "letmp")?.into(),
            11 => self.builder.build_int_compare(inkwell::IntPredicate::SGE, lhs_val, rhs_val, "getmp")?.into(),
            12 => self.builder.build_and(lhs_val, rhs_val, "andtmp")?.into(),
            13 => self.builder.build_or(lhs_val, rhs_val, "ortmp")?.into(),
            14 => self.builder.build_and(lhs_val, rhs_val, "andtmp")?.into(),
            15 => self.builder.build_or(lhs_val, rhs_val, "ortmp")?.into(),
            16 => self.builder.build_xor(lhs_val, rhs_val, "xortmp")?.into(),
            17 => self.builder.build_left_shift(lhs_val, rhs_val, "shltmp")?.into(),
            18 => self.builder.build_right_shift(lhs_val, rhs_val, false, "shrtmp")?.into(),
            19 => {
                let ptr = lhs_basic.into_pointer_value();
                self.builder.build_store(ptr, rhs_val)?;
                rhs_val.into()
            },
            _ => return Err(BackendError::InvalidOperator(node.data)),
        };
        
        Ok(Some(result))
    }

    fn lower_unop(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let child_offset = node.first_child;
        let operand_node = arena.get(child_offset).ok_or(BackendError::InvalidNode)?;
        let operand = self.lower_nodes(arena, child_offset)?.ok_or(BackendError::InvalidNode)?;
        
        let int_operand = operand.into_int_value();
        let result: BasicValueEnum<'ctx> = match node.data {
            1 => self.builder.build_int_neg(int_operand, "negtmp")?.into(),
            2 => self.builder.build_not(int_operand, "nottmp")?.into(),
            3 => {
                let all_ones = self.context.i32_type().const_int(u32::MAX as u64, false);
                self.builder.build_xor(int_operand, all_ones, "nottmp")?.into()
            },
            4 => return Err(BackendError::InvalidOperator(4)),
            5 => {
                let ptr = operand.into_pointer_value();
                self.builder.build_load(ptr, "deref")?.into()
            },
            _ => return Err(BackendError::InvalidOperator(node.data)),
        };

        Ok(Some(result))
    }

    fn lower_cond(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(None)
    }

    fn lower_call(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let first_child = arena.get(node.first_child);
        let func_name_offset = first_child.map(|c| NodeOffset(c.data));
        let mut args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
        
        // Start from second child (first child is the function name)
        let mut child_offset = NodeOffset::NULL;
        if let Some(c) = first_child {
            child_offset = c.next_sibling;
        }
        
        while child_offset.0 != 0 {
            let c = arena.get(child_offset);
            if let Some(arg_node) = c {
                if let Some(arg) = self.lower_nodes(arena, child_offset)? {
                    args.push(arg.into());
                }
                child_offset = arg_node.next_sibling;
            } else {
                break;
            }
        }
        
        let callee = func_name_offset.and_then(|offset| self.functions.get(&offset));
        match callee {
            Some(f) => {
                let result = self.builder.build_call(*f, &args, "calltmp")?;
                let _result = self.builder.build_call(*f, &args, "calltmp")?;
                // Call result handling - simplified for now
                Ok(None)
            }
            None => Err(BackendError::UndefinedFunction(func_name_offset.unwrap_or(NodeOffset::NULL))),
        }
    }

    fn lower_array_subscript(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(None)
    }

    fn lower_member(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(None)
    }

    fn lower_cast(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(None)
    }

    fn lower_sizeof(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(Some(self.context.i32_type().const_int(std::mem::size_of::<i32>() as u64, false).into()))
    }

    fn lower_comma(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let mut last_value: Option<BasicValueEnum> = None;
        let mut child_offset = node.first_child;
        while child_offset.0 != 0 {
            let c = arena.get(child_offset);
            if let Some(child_node) = c {
                last_value = self.lower_nodes(arena, child_offset)?;
                child_offset = child_node.next_sibling;
            } else {
                break;
            }
        }
        Ok(last_value)
    }

    fn lower_assign(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let lhs_offset = node.first_child;
        let lhs_opt = self.lower_nodes(arena, lhs_offset)?;
        let lhs_val = lhs_opt.ok_or(BackendError::InvalidNode)?;
        
        let rhs_val = if let Some(c) = arena.get(lhs_offset) {
            let rhs_offset = c.next_sibling;
            let rhs_opt = self.lower_nodes(arena, rhs_offset)?;
            rhs_opt.ok_or(BackendError::InvalidNode)?
        } else {
            return Err(BackendError::InvalidNode);
        };
        
        let ptr = lhs_val.into_pointer_value();
        self.builder.build_store(ptr, rhs_val)?;
        Ok(Some(rhs_val))
    }

    fn lower_num(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let value = node.data as u64;
        Ok(Some(self.context.i32_type().const_int(value, false).into()))
    }

    fn lower_float_const(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let bits = node.data;
        Ok(Some(self.context.f32_type().const_float(bits as f64).into()))
    }

    fn get_vector_width(&self, _node: NodeOffset) -> Option<u32> {
        if self.vectorization_hints.loop_vectorize {
            Some(self.vectorization_hints.vector_width)
        } else {
            None
        }
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
        // Optimization disabled - PassManager API changed in inkwell 0.9
        Ok(())
    }
}

#[derive(Debug)]
pub enum BackendError {
    UnknownNodeKind(u16),
    InvalidNode,
    UndefinedVariable(NodeOffset),
    UndefinedFunction(NodeOffset),
    InvalidOperator(u32),
    VerificationFailed(String),
    IoError(String),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            BackendError::UnknownNodeKind(k) => write!(f, "Unknown AST node kind: {}", k),
            BackendError::InvalidNode => write!(f, "Invalid AST node"),
            BackendError::UndefinedVariable(offset) => write!(f, "Undefined variable at offset: {}", offset.0),
            BackendError::UndefinedFunction(offset) => write!(f, "Undefined function at offset: {}", offset.0),
            BackendError::InvalidOperator(op) => write!(f, "Invalid operator code: {}", op),
            BackendError::VerificationFailed(msg) => write!(f, "LLVM verification failed: {}", msg),
            BackendError::IoError(msg) => write!(f, "I/O error: {}", msg),
        }
    }
}

impl std::error::Error for BackendError {}

impl From<inkwell::builder::BuilderError> for BackendError {
    fn from(_: inkwell::builder::BuilderError) -> Self {
        BackendError::InvalidNode
    }
}

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
        // Module should have some IR now (header comments etc)
        assert!(ir.contains("test_module"));
    }
}

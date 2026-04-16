use crate::arena::{Arena, CAstNode, NodeOffset, NodeFlags};
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::builder::Builder;
use inkwell::types::{BasicType, BasicTypeEnum, FunctionType};
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

        let result = match node.kind {
            /* Types - handled in declaration context */
            1 => self.lower_type(arena, node)?,           // AST_VOID
            2 => self.lower_type(arena, node)?,           // AST_INT
            3 => self.lower_type(arena, node)?,           // AST_CHAR
            4 => self.lower_type(arena, node)?,           // AST_STRUCT
            5 => self.lower_type(arena, node)?,           // AST_UNION
            6 => self.lower_type(arena, node)?,           // AST_ENUM
            7 => self.lower_type(arena, node)?,           // AST_PTR
            8 => self.lower_type(arena, node)?,           // AST_ARRAY
            9 => self.lower_type(arena, node)?,           // AST_FUNC

            /* Declarations */
            20 => self.lower_decl(arena, node)?,         // AST_DECL
            21 => self.lower_var_decl(arena, node)?,     // AST_VAR_DECL
            22 => self.lower_func_decl(arena, node)?,     // AST_FUNC_DECL
            23 => self.lower_func_def(arena, node)?,      // AST_FUNC_DEF
            24 => self.lower_param(arena, node)?,        // AST_PARAM
            25 => self.lower_struct_decl(arena, node)?,   // AST_STRUCT_DECL
            26 => self.lower_enum_const(arena, node)?,    // AST_ENUM_CONST

            /* Statements */
            40 => self.lower_compound(arena, node)?,      // AST_COMPOUND
            41 => self.lower_if(arena, node)?,            // AST_IF
            42 => self.lower_while(arena, node)?,         // AST_WHILE
            43 => self.lower_for(arena, node)?,           // AST_FOR
            44 => self.lower_return(arena, node)?,        // AST_RETURN
            45 => self.lower_expr_stmt(arena, node)?,     // AST_EXPR_STMT
            46 => self.lower_break(),                     // AST_BREAK
            47 => self.lower_continue(),                   // AST_CONTINUE
            48 => Ok(None),                               // AST_EMPTY
            50 => self.lower_switch(arena, node)?,        // AST_SWITCH

            /* Expressions */
            60 => self.lower_ident(arena, node)?,        // AST_IDENT
            61 => self.lower_int_const(arena, node)?,    // AST_INT_CONST
            62 => self.lower_char_const(arena, node)?,   // AST_CHAR_CONST
            63 => self.lower_string(arena, node)?,       // AST_STRING
            64 => self.lower_binop(arena, node)?,        // AST_BINOP
            65 => self.lower_unop(arena, node)?,         // AST_UNOP
            66 => self.lower_cond(arena, node)?,         // AST_COND
            67 => self.lower_call(arena, node)?,         // AST_CALL
            68 => self.lower_array_subscript(arena, node)?, // AST_ARRAY_SUBSCRIPT
            69 => self.lower_member(arena, node)?,       // AST_MEMBER
            70 => self.lower_cast(arena, node)?,         // AST_CAST
            71 => self.lower_sizeof(arena, node)?,       // AST_SIZEOF
            72 => self.lower_comma(arena, node)?,        // AST_COMMA
            73 => self.lower_assign(arena, node)?,       // AST_ASSIGN

            /* Literals */
            80 => self.lower_num(arena, node)?,          // AST_NUM
            81 => self.lower_string(arena, node)?,       // AST_STRING_LIT
            82 => self.lower_float_const(arena, node)?,  // AST_FLOAT_CONST
            83 => self.lower_type(arena, node)?,          // AST_FLOAT

            _ => Err(BackendError::UnknownNodeKind(node.kind)),
        };

        Ok(result)
    }

    fn lower_type(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicTypeEnum<'ctx>>, BackendError> {
        let child = arena.get(node.first_child).ok();
        match node.kind {
            1 => Ok(Some(self.context.i32_type().into())),          // AST_VOID
            2 => Ok(Some(self.context.i32_type().into())),          // AST_INT
            3 => Ok(Some(self.context.i8_type().into())),           // AST_CHAR
            4 | 5 => self.lower_struct_union_type(arena, node),    // AST_STRUCT/UNION
            6 => Ok(Some(self.context.i32_type().into())),          // AST_ENUM
            7 => self.lower_ptr_type(arena, node),                // AST_PTR
            8 => self.lower_array_type(arena, node),               // AST_ARRAY
            9 => self.lower_func_type(arena, node),                // AST_FUNC
            83 => Ok(Some(self.context.f32_type().into())),         // AST_FLOAT
            _ => Err(BackendError::UnknownNodeKind(node.kind)),
        }
    }

    fn lower_struct_union_type(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicTypeEnum<'ctx>>, BackendError> {
        let mut struct_fields: Vec<BasicTypeEnum> = Vec::new();
        let mut child = arena.get(node.first_child);
        while let Some(c) = child {
            if let Some(field_type) = self.lower_type(arena, &c)? {
                struct_fields.push(field_type);
            }
            child = arena.get(c.next_sibling);
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

    fn lower_func_type(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicTypeEnum<'ctx>>, BackendError> {
        let return_type = self.context.i32_type();
        let mut param_types: Vec<BasicTypeEnum> = Vec::new();
        let mut child = arena.get(node.first_child);
        while let Some(c) = child {
            if let Some(t) = self.lower_type(arena, &c)? {
                param_types.push(t);
            }
            child = arena.get(c.next_sibling);
        }
        let fn_type = return_type.fn_type(&param_types, false);
        Ok(Some(fn_type.into()))
    }

    fn lower_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let mut child = arena.get(node.first_child);
        while let Some(c) = child {
            self.lower_nodes(arena, arena.get(c).map(|n| n).ok())?;
            child = arena.get(c.next_sibling);
        }
        Ok(None)
    }

    fn lower_var_decl(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let name_offset = NodeOffset(node.data);
        let var_ptr = self.builder.build_alloca(self.context.i32_type(), "var");
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
            if let Some(_width) = self.get_vector_width(name_offset) {
                function.add_attribute(0, & inkwell::attributes::Attribute::get_named_enum(self.context, "vectorize.enable", ""));
            }
        }
        
        let mut child = arena.get(node.first_child);
        while let Some(c) = child {
            self.lower_nodes(arena, c)?;
            child = arena.get(c.next_sibling);
        }
        
        self.builder.build_return(Some(&self.context.i32_type().const_int(0, false)));
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
        let mut child = arena.get(node.first_child);
        while let Some(c) = child {
            last_value = self.lower_nodes(arena, c)?;
            child = arena.get(c.next_sibling);
        }
        Ok(last_value)
    }

    fn lower_if(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let mut child = arena.get(node.first_child);
        let cond = child.map(|c| self.lower_nodes(arena, c)).ok().flatten();
        child = child.and_then(|c| arena.get(c.next_sibling));
        let then_block = self.lower_nodes(arena, child.ok_or(BackendError::InvalidNode)?)?.is_some();
        child = child.and_then(|c| arena.get(c.next_sibling));
        let else_block = child.map(|c| self.lower_nodes(arena, c)).ok().flatten();
        
        Ok(None)
    }

    fn lower_while(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(None)
    }

    fn lower_for(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(None)
    }

    fn lower_return(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let child = arena.get(node.first_child);
        match child {
            Some(c) => {
                let value = self.lower_nodes(arena, c)?;
                if let Some(v) = value {
                    self.builder.build_return(Some(&v));
                }
            }
            None => {
                self.builder.build_return(None);
            }
        }
        Ok(None)
    }

    fn lower_expr_stmt(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let child = arena.get(node.first_child);
        match child {
            Some(c) => self.lower_nodes(arena, c),
            None => Ok(None),
        }
    }

    fn lower_break(&mut self) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        self.builder.build_unconditional_branch(self.context.append_basic_block(self.functions.values().next().unwrap(), ""));
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
            let value = self.builder.build_load(self.context.i32_type(), *ptr, "ident");
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
        Ok(Some(global.as_basic_value()))
    }

    fn lower_binop(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let mut child = arena.get(node.first_child);
        let lhs = child.map(|c| self.lower_nodes(arena, c)).ok().flatten();
        child = child.and_then(|c| arena.get(c.next_sibling));
        let rhs = child.map(|c| self.lower_nodes(arena, c)).ok().flatten();
        
        let lhs_val = lhs.ok_or(BackendError::InvalidNode)?;
        let rhs_val = rhs.ok_or(BackendError::InvalidNode)?;
        
        let result = match node.data {
            1 => self.builder.build_add(lhs_val, rhs_val, "addtmp"),   // OP_ADD
            2 => self.builder.build_sub(lhs_val, rhs_val, "subtmp"),   // OP_SUB
            3 => self.builder.build_mul(lhs_val, rhs_val, "multmp"),   // OP_MUL
            4 => self.builder.build_div(lhs_val, rhs_val, "divtmp"),   // OP_DIV
            5 => self.builder.build_rem(lhs_val, rhs_val, "modtmp"),   // OP_MOD
            6 => self.builder.build_int_compare(inkwell::IntPredicate::EQ, lhs_val, rhs_val, "eqtmp"),  // OP_EQ
            7 => self.builder.build_int_compare(inkwell::IntPredicate::NE, lhs_val, rhs_val, "netmp"),   // OP_NE
            8 => self.builder.build_int_compare(inkwell::IntPredicate::SLT, lhs_val, rhs_val, "lttmp"),  // OP_LT
            9 => self.builder.build_int_compare(inkwell::IntPredicate::SGT, lhs_val, rhs_val, "gttmp"),  // OP_GT
            10 => self.builder.build_int_compare(inkwell::IntPredicate::SLE, lhs_val, rhs_val, "letmp"), // OP_LE
            11 => self.builder.build_int_compare(inkwell::IntPredicate::SGE, lhs_val, rhs_val, "getmp"), // OP_GE
            12 => self.builder.build_and(lhs_val, rhs_val, "andtmp"),  // OP_AND
            13 => self.builder.build_or(lhs_val, rhs_val, "ortmp"),   // OP_OR
            14 => self.builder.build_and(lhs_val, rhs_val, "andtmp"),  // OP_BITAND
            15 => self.builder.build_or(lhs_val, rhs_val, "ortmp"),   // OP_BITOR
            16 => self.builder.build_xor(lhs_val, rhs_val, "xortmp"), // OP_XOR
            17 => self.builder.build_left_shift(lhs_val, rhs_val, "shltmp"),  // OP_SHL
            18 => self.builder.build_right_shift(lhs_val, rhs_val, "shrtmp"), // OP_SHR
            19 => self.builder.build_store(lhs_val, rhs_val),         // OP_ASSIGN
            _ => return Err(BackendError::InvalidOperator(node.data)),
        };
        
        Ok(Some(result))
    }

    fn lower_unop(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let child = arena.get(node.first_child).ok_or(BackendError::InvalidNode)?;
        let operand = self.lower_nodes(arena, child)?.ok_or(BackendError::InvalidNode)?;
        
        let result = match node.data {
            1 => self.builder.build_neg(operand, "negtmp"),    // OP_NEG
            2 => self.builder.build_not(operand, "nottmp"),    // OP_NOT
            3 => self.builder.build_not(operand, "nottmp"),    // OP_BITNOT
            4 => Err(BackendError::InvalidOperator(4)),        // OP_ADDR (not directly supported)
            5 => self.builder.build_load(self.context.i32_type(), operand, "deref"), // OP_DEREF
            _ => return Err(BackendError::InvalidOperator(node.data)),
        };
        
        Ok(Some(result))
    }

    fn lower_cond(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        Ok(None)
    }

    fn lower_call(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let func_name_offset = arena.get(node.first_child).map(|c| NodeOffset(c.data));
        let mut args: Vec<BasicValueEnum> = Vec::new();
        let mut child = arena.get(node.first_child).and_then(|c| arena.get(c.next_sibling));
        
        while let Some(c) = child {
            if let Some(arg) = self.lower_nodes(arena, c)? {
                args.push(arg);
            }
            child = arena.get(c.next_sibling);
        }
        
        let callee = func_name_offset.and_then(|offset| self.functions.get(&offset));
        match callee {
            Some(f) => {
                let result = self.builder.build_call(*f, &args, "calltmp");
                Ok(Some(result.try_as_basic_value().left().ok_or(BackendError::InvalidNode)?))
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
        let mut child = arena.get(node.first_child);
        while let Some(c) = child {
            last_value = self.lower_nodes(arena, c)?;
            child = arena.get(c.next_sibling);
        }
        Ok(last_value)
    }

    fn lower_assign(&mut self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let mut child = arena.get(node.first_child);
        let lhs = child.map(|c| self.lower_nodes(arena, c)).ok().flatten();
        child = child.and_then(|c| arena.get(c.next_sibling));
        let rhs = child.map(|c| self.lower_nodes(arena, c)).ok().flatten();
        
        let rhs_val = rhs.ok_or(BackendError::InvalidNode)?;
        if let Some(lhs_ptr) = lhs {
            self.builder.build_store(lhs_ptr, rhs_val);
        }
        Ok(Some(rhs_val))
    }

    fn lower_num(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let value = node.data as u64;
        Ok(Some(self.context.i32_type().const_int(value, false).into()))
    }

    fn lower_float_const(&self, arena: &Arena, node: &CAstNode) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let bits = node.data;
        Ok(Some(self.context.f32_type().const_float_raw_bits(bits as u64).into()))
    }

    fn get_vector_width(&self, _node: NodeOffset) -> Option<u32> {
        if self.vectorization_hints.loop_vectorize {
            Some(self.vectorization_hints.vector_width)
        } else {
            None
        }
    }

    pub fn dump_ir(&self) -> String {
        self.module.print_to_string()
    }

    pub fn verify(&self) -> Result<(), BackendError> {
        if self.module.verify().is_err() {
            return Err(BackendError::VerificationFailed(self.module.print_to_string()));
        }
        Ok(())
    }

    pub fn optimize(&self, opt_level: u32) -> Result<(), BackendError> {
        if opt_level > 0 {
            let pass_manager = self.context.create_pass_manager();
            pass_manager.add_instruction_combining_pass();
            if opt_level >= 2 {
                pass_manager.add_reassociation_pass();
                pass_manager.add_gvn_pass();
            }
            pass_manager.run_on(&self.module);
        }
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
        assert_eq!(backend.dump_ir(), "");
    }
}

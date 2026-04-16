use std::collections::HashMap;

use crate::arena::{Arena, CAstNode, NodeOffset};

use super::{
    CType, ParamType, StructMember, TypeId, TypeSystem,
};
#[allow(unused_imports)]
use super::TypeQualifiers;

#[derive(Debug)]
pub enum TypeError {
    UnknownType(String),
    TypeMismatch(TypeId, TypeId),
    InvalidOperator(u32, TypeId, TypeId),
    StructIncomplete(String),
    CircularTypedef(String),
    IncompatibleAssignment(TypeId, TypeId),
    TooManyArguments,
    TooFewArguments,
}

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeError::UnknownType(name) => write!(f, "Unknown type: {}", name),
            TypeError::TypeMismatch(a, b) => write!(f, "Type mismatch: {:?} vs {:?}", a, b),
            TypeError::InvalidOperator(op, a, b) => {
                write!(f, "Invalid operator {} for types {:?} and {:?}", op, a, b)
            }
            TypeError::StructIncomplete(name) => write!(f, "Incomplete struct: {}", name),
            TypeError::CircularTypedef(name) => write!(f, "Circular typedef: {}", name),
            TypeError::IncompatibleAssignment(a, b) => {
                write!(f, "Incompatible assignment: {:?} = {:?}", a, b)
            }
            TypeError::TooManyArguments => write!(f, "Too many arguments"),
            TypeError::TooFewArguments => write!(f, "Too few arguments"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Assign,
}

impl BinaryOp {
    pub fn from_u32(op: u32) -> Option<Self> {
        match op {
            1 => Some(BinaryOp::Add),
            2 => Some(BinaryOp::Sub),
            3 => Some(BinaryOp::Mul),
            4 => Some(BinaryOp::Div),
            5 => Some(BinaryOp::Mod),
            6 => Some(BinaryOp::Eq),
            7 => Some(BinaryOp::Ne),
            8 => Some(BinaryOp::Lt),
            9 => Some(BinaryOp::Le),
            10 => Some(BinaryOp::Gt),
            11 => Some(BinaryOp::Ge),
            12 => Some(BinaryOp::And),
            13 => Some(BinaryOp::Or),
            14 => Some(BinaryOp::BitAnd),
            15 => Some(BinaryOp::BitOr),
            16 => Some(BinaryOp::BitXor),
            17 => Some(BinaryOp::Shl),
            18 => Some(BinaryOp::Shr),
            19 => Some(BinaryOp::Assign),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
    AddrOf,
    Deref,
    PreInc,
    PreDec,
    PostInc,
    PostDec,
    Sizeof,
}

impl UnaryOp {
    pub fn from_u32(op: u32) -> Option<Self> {
        match op {
            20 => Some(UnaryOp::Neg),
            21 => Some(UnaryOp::Not),
            22 => Some(UnaryOp::BitNot),
            23 => Some(UnaryOp::AddrOf),
            24 => Some(UnaryOp::Deref),
            25 => Some(UnaryOp::PreInc),
            26 => Some(UnaryOp::PreDec),
            27 => Some(UnaryOp::PostInc),
            28 => Some(UnaryOp::PostDec),
            29 => Some(UnaryOp::Sizeof),
            _ => None,
        }
    }
}

pub struct TypeResolver<'a> {
    types: &'a mut TypeSystem,
    node_types: HashMap<NodeOffset, TypeId>,
    typedefs: HashMap<String, TypeId>,
    struct_defs: HashMap<String, TypeId>,
    errors: Vec<TypeError>,
}

impl<'a> TypeResolver<'a> {
    pub fn new(types: &'a mut TypeSystem) -> Self {
        Self {
            types,
            node_types: HashMap::new(),
            typedefs: HashMap::new(),
            struct_defs: HashMap::new(),
            errors: Vec::new(),
        }
    }

    pub fn resolve(&mut self, arena: &Arena, root: NodeOffset) -> Result<(), ()> {
        self.resolve_node(arena, root);
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(())
        }
    }

    fn resolve_node(&mut self, arena: &Arena, offset: NodeOffset) -> Option<TypeId> {
        let node = arena.get(offset)?;

        let result = match node.kind {
            1..=15 => self.resolve_expression(arena, node),
            83 | 84 => self.resolve_type_specifier(arena, node),
            85 => self.resolve_typedef_decl(arena, node),
            86 => self.resolve_struct_decl(arena, node),
            87 => self.resolve_union_decl(arena, node),
            88 => self.resolve_enum_decl(arena, node),
            89 => self.resolve_function_decl(arena, node),
            _ => self.resolve_children(arena, node),
        };

        if let Some(ty) = result {
            self.node_types.insert(offset, ty);
        }

        result
    }

    fn resolve_children(&mut self, arena: &Arena, node: &CAstNode) -> Option<TypeId> {
        let mut result = None;
        let mut child = node.first_child;
        while child.0 != 0 {
            result = self.resolve_node(arena, child);
            if let Some(child_node) = arena.get(child) {
                child = child_node.next_sibling;
            } else {
                break;
            }
        }
        result
    }

    fn resolve_expression(&mut self, arena: &Arena, node: &CAstNode) -> Option<TypeId> {
        let child = node.first_child;
        if child.0 == 0 {
            return Some(self.infer_literal_type(node));
        }

        let child_node = arena.get(child)?;
        let child_type = self.resolve_node(arena, child);

        match node.kind {
            1 | 2 | 3 | 4 | 5 => {
                let sibling = child_node.next_sibling;
                if sibling.0 != 0 {
                    let sibling_type = self.resolve_node(arena, sibling);
                    if let (Some(lhs), Some(rhs)) = (child_type, sibling_type) {
                        let op = node.kind as u32;
                        match self.check_binary_op(op, lhs, rhs) {
                            Ok(result_type) => return Some(result_type),
                            Err(e) => {
                                self.errors.push(e);
                                return Some(lhs);
                            }
                        }
                    }
                }
                child_type
            }
            20..=29 => {
                if let Some(operand) = child_type {
                    let op = node.kind as u32;
                    match self.check_unary_op(op, operand) {
                        Ok(result_type) => Some(result_type),
                        Err(e) => {
                            self.errors.push(e);
                            Some(operand)
                        }
                    }
                } else {
                    child_type
                }
            }
            6 => {
                let sibling = child_node.next_sibling;
                if sibling.0 != 0 {
                    if let Some(rhs_node) = arena.get(sibling) {
                        let rhs_child = rhs_node.first_child;
                        if rhs_child.0 != 0 {
                            let func_type = self.resolve_node(arena, rhs_child);
                            if let Some(CType::Function {
                                return_type,
                                params,
                                ..
                            }) = func_type.and_then(|t| self.types.get_type(t).cloned())
                            {
                                let mut arg_count = 0;
                                let mut arg = rhs_child;
                                while arg.0 != 0 {
                                    if let Some(arg_node) = arena.get(arg) {
                                        self.resolve_node(arena, arg);
                                        arg_count += 1;
                                        arg = arg_node.next_sibling;
                                    } else {
                                        break;
                                    }
                                }
                                if arg_count > params.len() {
                                    self.errors.push(TypeError::TooManyArguments);
                                } else if arg_count < params.len() {
                                    self.errors.push(TypeError::TooFewArguments);
                                }
                                return Some(return_type);
                            }
                        }
                    }
                }
                Some(TypeId::INT)
            }
            19 => {
                let sibling = child_node.next_sibling;
                if sibling.0 != 0 {
                    let lhs_t = child_type;
                    let rhs_t = self.resolve_node(arena, sibling);
                    if let (Some(lhs), Some(rhs)) = (lhs_t, rhs_t) {
                        match self.check_assignment(lhs, rhs) {
                            Ok(t) => return Some(t),
                            Err(e) => {
                                self.errors.push(e);
                                return Some(lhs);
                            }
                        }
                    }
                }
                child_type
            }
            _ => child_type,
        }
    }

    fn infer_literal_type(&self, node: &CAstNode) -> TypeId {
        match node.kind {
            11 => TypeId::INT,
            12 => TypeId::DOUBLE,
            13 => TypeId::CHAR,
            14 => TypeId::INT,
            15 => TypeId::INT,
            _ => TypeId::INT,
        }
    }

    fn resolve_type_specifier(&mut self, _arena: &Arena, node: &CAstNode) -> Option<TypeId> {
        match node.data {
            0 => Some(TypeId::INT),
            1 => Some(TypeId::VOID),
            2 => Some(TypeId::CHAR),
            3 => Some(TypeId::SHORT),
            4 => Some(TypeId::LONG),
            5 => Some(TypeId::FLOAT),
            6 => Some(TypeId::DOUBLE),
            7 => Some(TypeId::BOOL),
            8 => Some(TypeId::LONGLONG),
            _ => {
                let name = format!("type_{}", node.data);
                if let Some(&tid) = self.typedefs.get(&name) {
                    Some(tid)
                } else if let Some(&tid) = self.struct_defs.get(&name) {
                    Some(tid)
                } else {
                    self.errors.push(TypeError::UnknownType(name));
                    Some(TypeId::INT)
                }
            }
        }
    }

    fn resolve_typedef_decl(&mut self, arena: &Arena, node: &CAstNode) -> Option<TypeId> {
        let child = node.first_child;
        if child.0 != 0 {
            let type_node = arena.get(child)?;
            let underlying = self.resolve_node(arena, child);
            if let Some(underlying_id) = underlying {
                let name = format!("typedef_{}", type_node.data);
                let typedef_id = self.types.add_type(CType::Typedef {
                    name: name.clone(),
                    underlying: underlying_id,
                });
                self.typedefs.insert(name, typedef_id);
                return Some(typedef_id);
            }
        }
        None
    }

    fn resolve_struct_decl(&mut self, arena: &Arena, node: &CAstNode) -> Option<TypeId> {
        let name = if node.data != 0 {
            Some(format!("struct_{}", node.data))
        } else {
            None
        };

        let struct_id = self.types.add_type(CType::Struct {
            name: name.clone(),
            members: Vec::new(),
            size: 0,
            align: 0,
        });

        if let Some(ref n) = name {
            self.struct_defs.insert(n.clone(), struct_id);
        }

        let mut members = Vec::new();
        let mut child = node.first_child;
        while child.0 != 0 {
            if let Some(member_node) = arena.get(child) {
                let member_type = self.resolve_node(arena, child);
                if let Some(ty) = member_type {
                    let mname = format!("member_{}", member_node.data);
                    members.push(StructMember {
                        name: mname,
                        type_id: ty,
                        offset: 0,
                        bit_offset: None,
                        bit_width: None,
                    });
                }
                child = member_node.next_sibling;
            } else {
                break;
            }
        }

        if let Some(CType::Struct {
            members: ms,
            ..
        }) = self.types.types.get_mut(struct_id.0 as usize)
        {
            *ms = members;
        }

        self.types.compute_struct_layout(struct_id);
        Some(struct_id)
    }

    fn resolve_union_decl(&mut self, arena: &Arena, node: &CAstNode) -> Option<TypeId> {
        let name = if node.data != 0 {
            Some(format!("union_{}", node.data))
        } else {
            None
        };

        let union_id = self.types.add_type(CType::Union {
            name: name.clone(),
            members: Vec::new(),
            size: 0,
            align: 0,
        });

        if let Some(ref n) = name {
            self.struct_defs.insert(n.clone(), union_id);
        }

        let mut members = Vec::new();
        let mut child = node.first_child;
        while child.0 != 0 {
            if let Some(member_node) = arena.get(child) {
                let member_type = self.resolve_node(arena, child);
                if let Some(ty) = member_type {
                    let mname = format!("member_{}", member_node.data);
                    members.push(StructMember {
                        name: mname,
                        type_id: ty,
                        offset: 0,
                        bit_offset: None,
                        bit_width: None,
                    });
                }
                child = member_node.next_sibling;
            } else {
                break;
            }
        }

        if let Some(CType::Union {
            members: ms,
            ..
        }) = self.types.types.get_mut(union_id.0 as usize)
        {
            *ms = members;
        }

        self.types.compute_struct_layout(union_id);
        Some(union_id)
    }

    fn resolve_enum_decl(&mut self, _arena: &Arena, node: &CAstNode) -> Option<TypeId> {
        let name = if node.data != 0 {
            Some(format!("enum_{}", node.data))
        } else {
            None
        };

        let enum_id = self.types.add_type(CType::Enum {
            name: name.clone(),
            underlying: TypeId::INT,
        });

        if let Some(ref n) = name {
            self.struct_defs.insert(n.clone(), enum_id);
        }

        Some(enum_id)
    }

    fn resolve_function_decl(&mut self, arena: &Arena, node: &CAstNode) -> Option<TypeId> {
        let child = node.first_child;
        if child.0 == 0 {
            return None;
        }

        let return_type = self.resolve_node(arena, child);
        let mut params = Vec::new();
        let child_node = arena.get(child)?;
        let mut sibling = child_node.next_sibling;

        while sibling.0 != 0 {
            if let Some(param_node) = arena.get(sibling) {
                let param_type = self.resolve_node(arena, sibling);
                if let Some(ty) = param_type {
                    let pname = format!("param_{}", param_node.data);
                    params.push(ParamType {
                        name: Some(pname),
                        type_id: ty,
                    });
                }
                sibling = param_node.next_sibling;
            } else {
                break;
            }
        }

        let func_type = self.types.add_type(CType::Function {
            return_type: return_type.unwrap_or(TypeId::VOID),
            params,
            variadic: false,
        });

        Some(func_type)
    }

    pub fn get_node_type(&self, offset: NodeOffset) -> Option<TypeId> {
        self.node_types.get(&offset).copied()
    }

    pub fn check_binary_op(
        &self,
        op: u32,
        lhs: TypeId,
        rhs: TypeId,
    ) -> Result<TypeId, TypeError> {
        let lhs_base = self.types.resolve_typedef(lhs);
        let rhs_base = self.types.resolve_typedef(rhs);

        if let Some(bin_op) = BinaryOp::from_u32(op) {
            match bin_op {
                BinaryOp::Add => self.check_add(lhs_base, rhs_base),
                BinaryOp::Sub => self.check_sub(lhs_base, rhs_base),
                BinaryOp::Mul => self.check_arithmetic(lhs_base, rhs_base),
                BinaryOp::Div => self.check_arithmetic(lhs_base, rhs_base),
                BinaryOp::Mod => self.check_integer(lhs_base, rhs_base),
                BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                    self.check_comparison(lhs_base, rhs_base)
                }
                BinaryOp::And | BinaryOp::Or => self.check_logical(lhs_base, rhs_base),
                BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                    self.check_bitwise(lhs_base, rhs_base)
                }
                BinaryOp::Shl | BinaryOp::Shr => self.check_shift(lhs_base, rhs_base),
                BinaryOp::Assign => self.check_assignment(lhs_base, rhs_base),
            }
        } else {
            Err(TypeError::InvalidOperator(op, lhs, rhs))
        }
    }

    fn check_add(&self, lhs: TypeId, rhs: TypeId) -> Result<TypeId, TypeError> {
        if self.types.is_arithmetic(lhs) && self.types.is_arithmetic(rhs) {
            self.usual_arithmetic_conversions(lhs, rhs)
        } else if self.types.is_pointer(lhs) && self.types.is_integer(rhs) {
            Ok(lhs)
        } else if self.types.is_integer(lhs) && self.types.is_pointer(rhs) {
            Ok(rhs)
        } else {
            Err(TypeError::InvalidOperator(1, lhs, rhs))
        }
    }

    fn check_sub(&self, lhs: TypeId, rhs: TypeId) -> Result<TypeId, TypeError> {
        if self.types.is_arithmetic(lhs) && self.types.is_arithmetic(rhs) {
            self.usual_arithmetic_conversions(lhs, rhs)
        } else if self.types.is_pointer(lhs) && self.types.is_integer(rhs) {
            Ok(lhs)
        } else if self.types.is_pointer(lhs) && self.types.is_pointer(rhs) {
            Ok(TypeId::LONG)
        } else {
            Err(TypeError::InvalidOperator(2, lhs, rhs))
        }
    }

    fn check_arithmetic(&self, lhs: TypeId, rhs: TypeId) -> Result<TypeId, TypeError> {
        if self.types.is_arithmetic(lhs) && self.types.is_arithmetic(rhs) {
            self.usual_arithmetic_conversions(lhs, rhs)
        } else {
            Err(TypeError::InvalidOperator(0, lhs, rhs))
        }
    }

    fn check_integer(&self, lhs: TypeId, rhs: TypeId) -> Result<TypeId, TypeError> {
        if self.types.is_integer(lhs) && self.types.is_integer(rhs) {
            self.usual_arithmetic_conversions(lhs, rhs)
        } else {
            Err(TypeError::InvalidOperator(0, lhs, rhs))
        }
    }

    fn check_comparison(&self, lhs: TypeId, rhs: TypeId) -> Result<TypeId, TypeError> {
        if self.types.is_arithmetic(lhs) && self.types.is_arithmetic(rhs) {
            Ok(TypeId::INT)
        } else if self.types.is_pointer(lhs) && self.types.is_pointer(rhs) {
            Ok(TypeId::INT)
        } else {
            Err(TypeError::InvalidOperator(0, lhs, rhs))
        }
    }

    fn check_logical(&self, lhs: TypeId, rhs: TypeId) -> Result<TypeId, TypeError> {
        if self.types.is_scalar(lhs) && self.types.is_scalar(rhs) {
            Ok(TypeId::INT)
        } else {
            Err(TypeError::InvalidOperator(0, lhs, rhs))
        }
    }

    fn check_bitwise(&self, lhs: TypeId, rhs: TypeId) -> Result<TypeId, TypeError> {
        if self.types.is_integer(lhs) && self.types.is_integer(rhs) {
            self.usual_arithmetic_conversions(lhs, rhs)
        } else {
            Err(TypeError::InvalidOperator(0, lhs, rhs))
        }
    }

    fn check_shift(&self, lhs: TypeId, rhs: TypeId) -> Result<TypeId, TypeError> {
        if self.types.is_integer(lhs) && self.types.is_integer(rhs) {
            Ok(self.integer_promotion(lhs))
        } else {
            Err(TypeError::InvalidOperator(0, lhs, rhs))
        }
    }

    pub fn check_unary_op(&mut self, op: u32, operand: TypeId) -> Result<TypeId, TypeError> {
        let base = self.types.resolve_typedef(operand);

        if let Some(unary_op) = UnaryOp::from_u32(op) {
            match unary_op {
                UnaryOp::Neg => {
                    if self.types.is_arithmetic(base) {
                        Ok(base)
                    } else {
                        Err(TypeError::InvalidOperator(op, operand, TypeId::VOID))
                    }
                }
                UnaryOp::Not => {
                    if self.types.is_scalar(base) {
                        Ok(TypeId::INT)
                    } else {
                        Err(TypeError::InvalidOperator(op, operand, TypeId::VOID))
                    }
                }
                UnaryOp::BitNot => {
                    if self.types.is_integer(base) {
                        Ok(self.integer_promotion(base))
                    } else {
                        Err(TypeError::InvalidOperator(op, operand, TypeId::VOID))
                    }
                }
                UnaryOp::AddrOf => {
                    let ptr = self.types.add_type(CType::Pointer { base });
                    Ok(ptr)
                }
                UnaryOp::Deref => {
                    if let Some(base_type) = self.types.pointer_base(base) {
                        Ok(base_type)
                    } else {
                        Err(TypeError::InvalidOperator(op, operand, TypeId::VOID))
                    }
                }
                UnaryOp::PreInc | UnaryOp::PreDec | UnaryOp::PostInc | UnaryOp::PostDec => {
                    if self.types.is_arithmetic(base) || self.types.is_pointer(base) {
                        Ok(base)
                    } else {
                        Err(TypeError::InvalidOperator(op, operand, TypeId::VOID))
                    }
                }
                UnaryOp::Sizeof => Ok(TypeId::ULONG),
            }
        } else {
            Err(TypeError::InvalidOperator(op, operand, TypeId::VOID))
        }
    }

    pub fn implicit_conversion(&self, from: TypeId, to: TypeId) -> Result<TypeId, TypeError> {
        let from_base = self.types.resolve_typedef(from);
        let to_base = self.types.resolve_typedef(to);

        if from_base == to_base {
            return Ok(to_base);
        }

        if self.types.is_arithmetic(from_base) && self.types.is_arithmetic(to_base) {
            return Ok(to_base);
        }

        if self.types.is_integer(from_base) && self.types.is_pointer(to_base) {
            return Ok(to_base);
        }

        if self.types.is_pointer(from_base) && self.types.is_integer(to_base) {
            return Ok(to_base);
        }

        if self.types.is_pointer(from_base) && self.types.is_pointer(to_base) {
            return Ok(to_base);
        }

        Err(TypeError::TypeMismatch(from, to))
    }

    fn check_assignment(&self, lhs: TypeId, rhs: TypeId) -> Result<TypeId, TypeError> {
        let lhs_base = self.types.resolve_typedef(lhs);
        let rhs_base = self.types.resolve_typedef(rhs);

        if lhs_base == rhs_base {
            return Ok(lhs_base);
        }

        if self.types.is_arithmetic(lhs_base) && self.types.is_arithmetic(rhs_base) {
            return Ok(lhs_base);
        }

        if self.types.is_pointer(lhs_base) && self.types.is_pointer(rhs_base) {
            return Ok(lhs_base);
        }

        if self.types.is_integer(rhs_base)
            && (self.types.is_pointer(lhs_base) || self.types.is_pointer(rhs_base))
        {
            return Ok(lhs_base);
        }

        Err(TypeError::IncompatibleAssignment(lhs, rhs))
    }

    pub fn integer_promotion(&self, id: TypeId) -> TypeId {
        let base = self.types.resolve_typedef(id);
        match self.types.get_type(base) {
            Some(CType::Bool) => TypeId::INT,
            Some(CType::Char { .. }) => TypeId::INT,
            Some(CType::Short { signed }) => {
                if *signed {
                    TypeId::INT
                } else {
                    if self.types.size_of(TypeId::INT) >= self.types.size_of(base) {
                        TypeId::UINT
                    } else {
                        base
                    }
                }
            }
            _ => base,
        }
    }

    pub fn usual_arithmetic_conversions(
        &self,
        lhs: TypeId,
        rhs: TypeId,
    ) -> Result<TypeId, TypeError> {
        let lhs_promoted = self.integer_promotion(lhs);
        let rhs_promoted = self.integer_promotion(rhs);

        if lhs_promoted == rhs_promoted {
            return Ok(lhs_promoted);
        }

        let lhs_base = self.types.resolve_typedef(lhs_promoted);
        let rhs_base = self.types.resolve_typedef(rhs_promoted);

        if self.types.is_floating(lhs_base) || self.types.is_floating(rhs_base) {
            let lrank = self.types.integer_rank(lhs_base);
            let rrank = self.types.integer_rank(rhs_base);
            return Ok(if lrank >= rrank {
                lhs_base
            } else {
                rhs_base
            });
        }

        let l_signed = self.types.is_signed(lhs_base);
        let r_signed = self.types.is_signed(rhs_base);

        if l_signed == r_signed {
            let lrank = self.types.integer_rank(lhs_base);
            let rrank = self.types.integer_rank(rhs_base);
            return Ok(if lrank >= rrank {
                lhs_base
            } else {
                rhs_base
            });
        }

        let unsigned_rank = if l_signed {
            self.types.integer_rank(rhs_base)
        } else {
            self.types.integer_rank(lhs_base)
        };
        let signed_rank = if l_signed {
            self.types.integer_rank(lhs_base)
        } else {
            self.types.integer_rank(rhs_base)
        };
        let unsigned_type = if l_signed { rhs_base } else { lhs_base };
        let signed_type = if l_signed { lhs_base } else { rhs_base };

        if unsigned_rank >= signed_rank {
            return Ok(unsigned_type);
        }

        if self.types.size_of(signed_type) > self.types.size_of(unsigned_type) {
            return Ok(signed_type);
        }

        let unsigned_wider = match unsigned_type {
            x if x == TypeId::INT => TypeId::LONG,
            x if x == TypeId::UINT => TypeId::ULONG,
            x if x == TypeId::LONG => TypeId::LONGLONG,
            x if x == TypeId::ULONG => TypeId::ULONGLONG,
            _ => unsigned_type,
        };
        Ok(unsigned_wider)
    }

    pub fn errors(&self) -> &[TypeError] {
        &self.errors
    }

    pub fn register_typedef(&mut self, name: String, type_id: TypeId) {
        let typedef_id = self.types.add_type(CType::Typedef {
            name: name.clone(),
            underlying: type_id,
        });
        self.typedefs.insert(name, typedef_id);
    }

    pub fn register_struct(&mut self, name: String, members: Vec<StructMember>) -> TypeId {
        let struct_id = self.types.add_type(CType::Struct {
            name: Some(name.clone()),
            members,
            size: 0,
            align: 0,
        });
        self.types.compute_struct_layout(struct_id);
        self.struct_defs.insert(name, struct_id);
        struct_id
    }

    pub fn register_union(&mut self, name: String, members: Vec<StructMember>) -> TypeId {
        let union_id = self.types.add_type(CType::Union {
            name: Some(name.clone()),
            members,
            size: 0,
            align: 0,
        });
        self.types.compute_struct_layout(union_id);
        self.struct_defs.insert(name, union_id);
        union_id
    }

    pub fn register_enum(&mut self, name: String, underlying: TypeId) -> TypeId {
        let enum_id = self.types.add_type(CType::Enum {
            name: Some(name.clone()),
            underlying,
        });
        self.struct_defs.insert(name, enum_id);
        enum_id
    }

    pub fn lookup_typedef(&self, name: &str) -> Option<TypeId> {
        self.typedefs.get(name).copied()
    }

    pub fn lookup_struct(&self, name: &str) -> Option<TypeId> {
        self.struct_defs.get(name).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_integer_promotion_char() {
        let mut ts = TypeSystem::new();
        let resolver = TypeResolver::new(&mut ts);
        let promoted = resolver.integer_promotion(TypeId::CHAR);
        assert_eq!(promoted, TypeId::INT);
    }

    #[test]
    fn test_integer_promotion_short() {
        let mut ts = TypeSystem::new();
        let resolver = TypeResolver::new(&mut ts);
        let promoted = resolver.integer_promotion(TypeId::SHORT);
        assert_eq!(promoted, TypeId::INT);
    }

    #[test]
    fn test_integer_promotion_int() {
        let mut ts = TypeSystem::new();
        let resolver = TypeResolver::new(&mut ts);
        let promoted = resolver.integer_promotion(TypeId::INT);
        assert_eq!(promoted, TypeId::INT);
    }

    #[test]
    fn test_usual_arithmetic_int_int() {
        let mut ts = TypeSystem::new();
        let resolver = TypeResolver::new(&mut ts);
        let result = resolver
            .usual_arithmetic_conversions(TypeId::INT, TypeId::INT)
            .unwrap();
        assert_eq!(result, TypeId::INT);
    }

    #[test]
    fn test_usual_arithmetic_int_long() {
        let mut ts = TypeSystem::new();
        let resolver = TypeResolver::new(&mut ts);
        let result = resolver
            .usual_arithmetic_conversions(TypeId::INT, TypeId::LONG)
            .unwrap();
        assert_eq!(result, TypeId::LONG);
    }

    #[test]
    fn test_usual_arithmetic_int_float() {
        let mut ts = TypeSystem::new();
        let resolver = TypeResolver::new(&mut ts);
        let result = resolver
            .usual_arithmetic_conversions(TypeId::INT, TypeId::FLOAT)
            .unwrap();
        assert_eq!(result, TypeId::FLOAT);
    }

    #[test]
    fn test_usual_arithmetic_float_double() {
        let mut ts = TypeSystem::new();
        let resolver = TypeResolver::new(&mut ts);
        let result = resolver
            .usual_arithmetic_conversions(TypeId::FLOAT, TypeId::DOUBLE)
            .unwrap();
        assert_eq!(result, TypeId::DOUBLE);
    }

    #[test]
    fn test_usual_arithmetic_long_longlong() {
        let mut ts = TypeSystem::new();
        let resolver = TypeResolver::new(&mut ts);
        let result = resolver
            .usual_arithmetic_conversions(TypeId::LONG, TypeId::LONGLONG)
            .unwrap();
        assert_eq!(result, TypeId::LONGLONG);
    }

    #[test]
    fn test_binary_op_add_arithmetic() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_binary_op(1, TypeId::INT, TypeId::INT).unwrap();
        assert_eq!(result, TypeId::INT);
    }

    #[test]
    fn test_binary_op_add_pointer_int() {
        let mut ts = TypeSystem::new();
        let int_ptr = ts.add_type(CType::Pointer {
            base: TypeId::INT,
        });
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_binary_op(1, int_ptr, TypeId::INT).unwrap();
        assert_eq!(result, int_ptr);
    }

    #[test]
    fn test_binary_op_sub_pointer_pointer() {
        let mut ts = TypeSystem::new();
        let int_ptr = ts.add_type(CType::Pointer {
            base: TypeId::INT,
        });
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver
            .check_binary_op(2, int_ptr, int_ptr)
            .unwrap();
        assert_eq!(result, TypeId::LONG);
    }

    #[test]
    fn test_binary_op_mul() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_binary_op(3, TypeId::INT, TypeId::INT).unwrap();
        assert_eq!(result, TypeId::INT);
    }

    #[test]
    fn test_binary_op_comparison() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_binary_op(8, TypeId::INT, TypeId::INT).unwrap();
        assert_eq!(result, TypeId::INT);
    }

    #[test]
    fn test_binary_op_logical_and() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_binary_op(12, TypeId::INT, TypeId::INT).unwrap();
        assert_eq!(result, TypeId::INT);
    }

    #[test]
    fn test_unary_op_neg() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_unary_op(20, TypeId::INT).unwrap();
        assert_eq!(result, TypeId::INT);
    }

    #[test]
    fn test_unary_op_not() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_unary_op(21, TypeId::INT).unwrap();
        assert_eq!(result, TypeId::INT);
    }

    #[test]
    fn test_unary_op_addr_of() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_unary_op(23, TypeId::INT).unwrap();
        assert!(ts.is_pointer(result));
        assert_eq!(ts.pointer_base(result), Some(TypeId::INT));
    }

    #[test]
    fn test_unary_op_deref() {
        let mut ts = TypeSystem::new();
        let int_ptr = ts.add_type(CType::Pointer {
            base: TypeId::INT,
        });
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_unary_op(24, int_ptr).unwrap();
        assert_eq!(result, TypeId::INT);
    }

    #[test]
    fn test_unary_op_sizeof() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_unary_op(29, TypeId::INT).unwrap();
        assert_eq!(result, TypeId::ULONG);
    }

    #[test]
    fn test_assignment_compatible() {
        let mut ts = TypeSystem::new();
        let resolver = TypeResolver::new(&mut ts);
        let result = resolver
            .check_assignment(TypeId::INT, TypeId::INT)
            .unwrap();
        assert_eq!(result, TypeId::INT);
    }

    #[test]
    fn test_assignment_implicit_conversion() {
        let mut ts = TypeSystem::new();
        let resolver = TypeResolver::new(&mut ts);
        let result = resolver
            .check_assignment(TypeId::LONG, TypeId::INT)
            .unwrap();
        assert_eq!(result, TypeId::LONG);
    }

    #[test]
    fn test_assignment_pointer_compatible() {
        let mut ts = TypeSystem::new();
        let int_ptr1 = ts.add_type(CType::Pointer {
            base: TypeId::INT,
        });
        let int_ptr2 = ts.add_type(CType::Pointer {
            base: TypeId::INT,
        });
        let resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_assignment(int_ptr1, int_ptr2).unwrap();
        assert_eq!(result, int_ptr1);
    }

    #[test]
    fn test_implicit_conversion_arithmetic() {
        let mut ts = TypeSystem::new();
        let resolver = TypeResolver::new(&mut ts);
        let result = resolver
            .implicit_conversion(TypeId::INT, TypeId::DOUBLE)
            .unwrap();
        assert_eq!(result, TypeId::DOUBLE);
    }

    #[test]
    fn test_error_collection() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_binary_op(1, TypeId::VOID, TypeId::VOID);
        assert!(result.is_err());
        resolver.errors.push(TypeError::TypeMismatch(TypeId::VOID, TypeId::INT));
        assert!(!resolver.errors().is_empty());
    }

    #[test]
    fn test_register_and_lookup_typedef() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        resolver.register_typedef("size_t".to_string(), TypeId::ULONG);
        let found = resolver.lookup_typedef("size_t");
        assert!(found.is_some());
    }

    #[test]
    fn test_register_struct_with_members() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let members = vec![
            StructMember {
                name: "x".to_string(),
                type_id: TypeId::INT,
                offset: 0,
                bit_offset: None,
                bit_width: None,
            },
            StructMember {
                name: "y".to_string(),
                type_id: TypeId::DOUBLE,
                offset: 0,
                bit_offset: None,
                bit_width: None,
            },
        ];
        let struct_id = resolver.register_struct("Point".to_string(), members);
        assert_eq!(ts.size_of(struct_id), 16);
        assert_eq!(ts.align_of(struct_id), 8);
    }

    #[test]
    fn test_register_union_with_members() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let members = vec![
            StructMember {
                name: "i".to_string(),
                type_id: TypeId::INT,
                offset: 0,
                bit_offset: None,
                bit_width: None,
            },
            StructMember {
                name: "d".to_string(),
                type_id: TypeId::DOUBLE,
                offset: 0,
                bit_offset: None,
                bit_width: None,
            },
        ];
        let union_id = resolver.register_union("Data".to_string(), members);
        assert_eq!(ts.size_of(union_id), 8);
        assert_eq!(ts.align_of(union_id), 8);
    }

    #[test]
    fn test_register_enum() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let enum_id = resolver.register_enum("Color".to_string(), TypeId::INT);
        assert_eq!(ts.size_of(enum_id), 4);
        assert!(ts.is_integer(enum_id));
    }

    #[test]
    fn test_binary_op_shift() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_binary_op(17, TypeId::INT, TypeId::INT).unwrap();
        assert_eq!(result, TypeId::INT);
    }

    #[test]
    fn test_binary_op_bitwise() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver
            .check_binary_op(14, TypeId::INT, TypeId::INT)
            .unwrap();
        assert_eq!(result, TypeId::INT);
    }

    #[test]
    fn test_unary_op_neg_float() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_unary_op(20, TypeId::DOUBLE).unwrap();
        assert_eq!(result, TypeId::DOUBLE);
    }

    #[test]
    fn test_unary_op_deref_invalid() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_unary_op(24, TypeId::INT);
        assert!(result.is_err());
    }

    #[test]
    fn test_usual_arithmetic_unsigned_signed() {
        let mut ts = TypeSystem::new();
        let resolver = TypeResolver::new(&mut ts);
        let result = resolver
            .usual_arithmetic_conversions(TypeId::UINT, TypeId::INT)
            .unwrap();
        assert_eq!(result, TypeId::UINT);
    }

    #[test]
    fn test_usual_arithmetic_double_long_double() {
        let mut ts = TypeSystem::new();
        let resolver = TypeResolver::new(&mut ts);
        let result = resolver
            .usual_arithmetic_conversions(TypeId::DOUBLE, TypeId::LONGDOUBLE)
            .unwrap();
        assert_eq!(result, TypeId::LONGDOUBLE);
    }

    #[test]
    fn test_check_add_invalid() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_binary_op(1, TypeId::VOID, TypeId::VOID);
        assert!(result.is_err());
    }

    #[test]
    fn test_check_mod_integer_only() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_binary_op(5, TypeId::INT, TypeId::INT).unwrap();
        assert_eq!(result, TypeId::INT);
        let result2 = resolver.check_binary_op(5, TypeId::FLOAT, TypeId::FLOAT);
        assert!(result2.is_err());
    }

    #[test]
    fn test_struct_member_offsets() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let members = vec![
            StructMember {
                name: "a".to_string(),
                type_id: TypeId::CHAR,
                offset: 0,
                bit_offset: None,
                bit_width: None,
            },
            StructMember {
                name: "b".to_string(),
                type_id: TypeId::INT,
                offset: 0,
                bit_offset: None,
                bit_width: None,
            },
        ];
        let struct_id = resolver.register_struct("S".to_string(), members);
        if let Some(CType::Struct { members, .. }) = ts.get_type(struct_id) {
            assert_eq!(members[0].offset, 0);
            assert_eq!(members[1].offset, 4);
        } else {
            panic!("Expected struct type");
        }
    }

    #[test]
    fn test_function_type_param_count() {
        let mut ts = TypeSystem::new();
        let func = ts.add_type(CType::Function {
            return_type: TypeId::VOID,
            params: vec![
                ParamType {
                    name: Some("a".to_string()),
                    type_id: TypeId::INT,
                },
                ParamType {
                    name: Some("b".to_string()),
                    type_id: TypeId::INT,
                },
            ],
            variadic: false,
        });
        if let Some(CType::Function { params, .. }) = ts.get_type(func) {
            assert_eq!(params.len(), 2);
        } else {
            panic!("Expected function type");
        }
    }

    #[test]
    fn test_qualifier_strip_nested() {
        let mut ts = TypeSystem::new();
        let cv_int = ts.add_type(CType::Qualified {
            qualifiers: TypeQualifiers::CONST | TypeQualifiers::VOLATILE,
            base: TypeId::INT,
        });
        let resolver = TypeResolver::new(&mut ts);
        let stripped = resolver.types.strip_qualifiers(cv_int);
        assert_eq!(stripped, TypeId::INT);
    }

    #[test]
    fn test_typedef_resolution_through_qualifiers() {
        let mut ts = TypeSystem::new();
        let my_int = ts.add_type(CType::Typedef {
            name: "my_int".to_string(),
            underlying: TypeId::INT,
        });
        let const_my_int = ts.add_type(CType::Qualified {
            qualifiers: TypeQualifiers::CONST,
            base: my_int,
        });
        let resolved = ts.resolve_typedef(const_my_int);
        assert_eq!(resolved, TypeId::INT);
    }

    #[test]
    fn test_bitfield_struct_layout() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let members = vec![
            StructMember {
                name: "a".to_string(),
                type_id: TypeId::INT,
                offset: 0,
                bit_offset: Some(0),
                bit_width: Some(4),
            },
            StructMember {
                name: "b".to_string(),
                type_id: TypeId::INT,
                offset: 0,
                bit_offset: Some(0),
                bit_width: Some(4),
            },
            StructMember {
                name: "c".to_string(),
                type_id: TypeId::INT,
                offset: 0,
                bit_offset: Some(0),
                bit_width: Some(8),
            },
        ];
        let struct_id = resolver.register_struct("BF".to_string(), members);
        assert_eq!(ts.size_of(struct_id), 4);
    }

    #[test]
    fn test_unary_op_pre_inc() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_unary_op(25, TypeId::INT).unwrap();
        assert_eq!(result, TypeId::INT);
    }

    #[test]
    fn test_unary_op_bitnot() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_unary_op(22, TypeId::INT).unwrap();
        assert_eq!(result, TypeId::INT);
    }

    #[test]
    fn test_binary_op_invalid() {
        let mut ts = TypeSystem::new();
        let mut resolver = TypeResolver::new(&mut ts);
        let result = resolver.check_binary_op(99, TypeId::INT, TypeId::INT);
        assert!(result.is_err());
    }
}

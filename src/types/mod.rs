use std::collections::HashMap;
use std::hash::{Hash, Hasher};

#[allow(unused_imports)]
use crate::arena::{Arena, CAstNode, NodeOffset};

pub mod resolve;
pub use resolve::{TypeError, TypeResolver};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub u32);

impl TypeId {
    pub const VOID: TypeId = TypeId(0);
    pub const BOOL: TypeId = TypeId(1);
    pub const CHAR: TypeId = TypeId(2);
    pub const SCHAR: TypeId = TypeId(3);
    pub const UCHAR: TypeId = TypeId(4);
    pub const SHORT: TypeId = TypeId(5);
    pub const USHORT: TypeId = TypeId(6);
    pub const INT: TypeId = TypeId(7);
    pub const UINT: TypeId = TypeId(8);
    pub const LONG: TypeId = TypeId(9);
    pub const ULONG: TypeId = TypeId(10);
    pub const LONGLONG: TypeId = TypeId(11);
    pub const ULONGLONG: TypeId = TypeId(12);
    pub const FLOAT: TypeId = TypeId(13);
    pub const DOUBLE: TypeId = TypeId(14);
    pub const LONGDOUBLE: TypeId = TypeId(15);
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CType {
    Void,
    Bool,
    Char {
        signed: bool,
    },
    Short {
        signed: bool,
    },
    Int {
        signed: bool,
    },
    Long {
        signed: bool,
    },
    LongLong {
        signed: bool,
    },
    Float,
    Double,
    LongDouble,
    Pointer {
        base: TypeId,
    },
    Array {
        element: TypeId,
        size: Option<u64>,
    },
    Struct {
        name: Option<String>,
        members: Vec<StructMember>,
        size: u64,
        align: u64,
    },
    Union {
        name: Option<String>,
        members: Vec<StructMember>,
        size: u64,
        align: u64,
    },
    Enum {
        name: Option<String>,
        underlying: TypeId,
    },
    Function {
        return_type: TypeId,
        params: Vec<ParamType>,
        variadic: bool,
    },
    Typedef {
        name: String,
        underlying: TypeId,
    },
    Qualified {
        qualifiers: TypeQualifiers,
        base: TypeId,
    },
}

#[derive(Debug, Clone)]
pub struct StructMember {
    pub name: String,
    pub type_id: TypeId,
    pub offset: u64,
    pub bit_offset: Option<u32>,
    pub bit_width: Option<u32>,
}

impl PartialEq for StructMember {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.type_id == other.type_id
            && self.offset == other.offset
            && self.bit_offset == other.bit_offset
            && self.bit_width == other.bit_width
    }
}

impl Eq for StructMember {}

impl Hash for StructMember {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.type_id.hash(state);
        self.offset.hash(state);
        self.bit_offset.hash(state);
        self.bit_width.hash(state);
    }
}

#[derive(Debug, Clone)]
pub struct ParamType {
    pub name: Option<String>,
    pub type_id: TypeId,
}

impl PartialEq for ParamType {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.type_id == other.type_id
    }
}

impl Eq for ParamType {}

impl Hash for ParamType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.type_id.hash(state);
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct TypeQualifiers: u8 {
        const CONST = 0x01;
        const VOLATILE = 0x02;
        const RESTRICT = 0x04;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum TypeSignature {
    Void,
    Bool,
    Char(bool),
    Short(bool),
    Int(bool),
    Long(bool),
    LongLong(bool),
    Float,
    Double,
    LongDouble,
    Pointer(TypeId),
    Array(TypeId, Option<u64>),
    Function(TypeId, Vec<ParamType>, bool),
    Enum(Option<String>, TypeId),
    Typedef(String, TypeId),
    Qualified(TypeQualifiers, TypeId),
}

impl TypeSignature {
    fn from_ctype(ty: &CType) -> Option<Self> {
        match ty {
            CType::Void => Some(TypeSignature::Void),
            CType::Bool => Some(TypeSignature::Bool),
            CType::Char { signed } => Some(TypeSignature::Char(*signed)),
            CType::Short { signed } => Some(TypeSignature::Short(*signed)),
            CType::Int { signed } => Some(TypeSignature::Int(*signed)),
            CType::Long { signed } => Some(TypeSignature::Long(*signed)),
            CType::LongLong { signed } => Some(TypeSignature::LongLong(*signed)),
            CType::Float => Some(TypeSignature::Float),
            CType::Double => Some(TypeSignature::Double),
            CType::LongDouble => Some(TypeSignature::LongDouble),
            CType::Pointer { base } => Some(TypeSignature::Pointer(*base)),
            CType::Array { element, size } => Some(TypeSignature::Array(*element, *size)),
            CType::Function {
                return_type,
                params,
                variadic,
            } => Some(TypeSignature::Function(
                *return_type,
                params.clone(),
                *variadic,
            )),
            CType::Enum { name, underlying } => {
                Some(TypeSignature::Enum(name.clone(), *underlying))
            }
            CType::Typedef { name, underlying } => {
                Some(TypeSignature::Typedef(name.clone(), *underlying))
            }
            CType::Qualified { qualifiers, base } => {
                Some(TypeSignature::Qualified(*qualifiers, *base))
            }
            CType::Struct { .. } | CType::Union { .. } => None,
        }
    }
}

struct LayoutInfo {
    size: u64,
    align: u64,
    members: Vec<StructMember>,
}

pub struct TypeSystem {
    types: Vec<CType>,
    type_cache: HashMap<TypeSignature, TypeId>,
}

impl TypeSystem {
    pub fn new() -> Self {
        let mut types = Vec::with_capacity(256);

        types.push(CType::Void);
        types.push(CType::Bool);
        types.push(CType::Char { signed: false });
        types.push(CType::Char { signed: true });
        types.push(CType::Char { signed: false });
        types.push(CType::Short { signed: true });
        types.push(CType::Short { signed: false });
        types.push(CType::Int { signed: true });
        types.push(CType::Int { signed: false });
        types.push(CType::Long { signed: true });
        types.push(CType::Long { signed: false });
        types.push(CType::LongLong { signed: true });
        types.push(CType::LongLong { signed: false });
        types.push(CType::Float);
        types.push(CType::Double);
        types.push(CType::LongDouble);

        let mut type_cache = HashMap::new();
        for i in 0..16u32 {
            if let Some(sig) = TypeSignature::from_ctype(&types[i as usize]) {
                type_cache.insert(sig, TypeId(i));
            }
        }

        Self { types, type_cache }
    }

    pub fn get_type(&self, id: TypeId) -> Option<&CType> {
        self.types.get(id.0 as usize)
    }

    pub fn add_type(&mut self, ty: CType) -> TypeId {
        if let Some(sig) = TypeSignature::from_ctype(&ty) {
            if let Some(&existing_id) = self.type_cache.get(&sig) {
                return existing_id;
            }
        }

        let id = TypeId(self.types.len() as u32);
        if let Some(sig) = TypeSignature::from_ctype(&ty) {
            self.type_cache.insert(sig, id);
        }
        self.types.push(ty);
        id
    }

    pub fn size_of(&self, id: TypeId) -> u64 {
        match self.get_type(id) {
            Some(ty) => match ty {
                CType::Void => 0,
                CType::Bool => 1,
                CType::Char { .. } => 1,
                CType::Short { .. } => 2,
                CType::Int { .. } => 4,
                CType::Long { .. } => 8,
                CType::LongLong { .. } => 8,
                CType::Float => 4,
                CType::Double => 8,
                CType::LongDouble => 16,
                CType::Pointer { .. } => 8,
                CType::Array { element, size } => {
                    let elem_size = self.size_of(*element);
                    match size {
                        Some(n) => elem_size * n,
                        None => 0,
                    }
                }
                CType::Struct { size, .. } => *size,
                CType::Union { size, .. } => *size,
                CType::Enum { underlying, .. } => self.size_of(*underlying),
                CType::Function { .. } => 0,
                CType::Typedef { underlying, .. } => self.size_of(*underlying),
                CType::Qualified { base, .. } => self.size_of(*base),
            },
            None => 0,
        }
    }

    pub fn align_of(&self, id: TypeId) -> u64 {
        match self.get_type(id) {
            Some(ty) => match ty {
                CType::Void => 1,
                CType::Bool => 1,
                CType::Char { .. } => 1,
                CType::Short { .. } => 2,
                CType::Int { .. } => 4,
                CType::Long { .. } => 8,
                CType::LongLong { .. } => 8,
                CType::Float => 4,
                CType::Double => 8,
                CType::LongDouble => 16,
                CType::Pointer { .. } => 8,
                CType::Array { element, .. } => self.align_of(*element),
                CType::Struct { align, .. } => *align,
                CType::Union { align, .. } => *align,
                CType::Enum { underlying, .. } => self.align_of(*underlying),
                CType::Function { .. } => 1,
                CType::Typedef { underlying, .. } => self.align_of(*underlying),
                CType::Qualified { base, .. } => self.align_of(*base),
            },
            None => 1,
        }
    }

    pub fn is_integer(&self, id: TypeId) -> bool {
        match self.get_type(id) {
            Some(ty) => match ty {
                CType::Bool
                | CType::Char { .. }
                | CType::Short { .. }
                | CType::Int { .. }
                | CType::Long { .. }
                | CType::LongLong { .. } => true,
                CType::Enum { .. } => true,
                CType::Qualified { base, .. } => self.is_integer(*base),
                CType::Typedef { underlying, .. } => self.is_integer(*underlying),
                _ => false,
            },
            None => false,
        }
    }

    pub fn is_floating(&self, id: TypeId) -> bool {
        matches!(
            self.get_type(id),
            Some(CType::Float | CType::Double | CType::LongDouble)
        )
    }

    pub fn is_pointer(&self, id: TypeId) -> bool {
        matches!(self.get_type(id), Some(CType::Pointer { .. }))
    }

    pub fn is_arithmetic(&self, id: TypeId) -> bool {
        self.is_integer(id) || self.is_floating(id)
    }

    pub fn is_scalar(&self, id: TypeId) -> bool {
        self.is_arithmetic(id) || self.is_pointer(id)
    }

    pub fn resolve_typedef(&self, id: TypeId) -> TypeId {
        let mut current = id;
        let mut visited = vec![];
        loop {
            if visited.contains(&current) {
                return current;
            }
            visited.push(current);
            match self.get_type(current) {
                Some(CType::Typedef { underlying, .. }) => {
                    current = *underlying;
                }
                Some(CType::Qualified { base, .. }) => {
                    current = *base;
                }
                _ => return current,
            }
        }
    }

    pub fn strip_qualifiers(&self, id: TypeId) -> TypeId {
        let mut current = id;
        loop {
            match self.get_type(current) {
                Some(CType::Qualified { base, .. }) => {
                    current = *base;
                }
                _ => return current,
            }
        }
    }

    pub fn compute_struct_layout(&mut self, id: TypeId) {
        let (members, align, is_union) = match self.get_type(id) {
            Some(CType::Struct { members, align, .. }) => (members.clone(), *align, false),
            Some(CType::Union { members, align, .. }) => (members.clone(), *align, true),
            _ => return,
        };

        if align == 0 {
            let computed = self.compute_layout_info(&members, is_union);
            self.update_struct_layout(id, computed.size, computed.align, computed.members);
        }
    }

    fn compute_layout_info(&self, members: &[StructMember], is_union: bool) -> LayoutInfo {
        let mut offset: u64 = 0;
        let mut max_align: u64 = 1;
        let mut new_members = Vec::new();

        if is_union {
            let mut max_size: u64 = 0;
            for member in members {
                let m_align = self.align_of(member.type_id);
                let m_size = self.size_of(member.type_id);
                if m_align > max_align {
                    max_align = m_align;
                }
                if m_size > max_size {
                    max_size = m_size;
                }
                new_members.push(StructMember {
                    name: member.name.clone(),
                    type_id: member.type_id,
                    offset: 0,
                    bit_offset: member.bit_offset,
                    bit_width: member.bit_width,
                });
            }
            let rounded = if max_align > 0 {
                (max_size + max_align - 1) / max_align * max_align
            } else {
                max_size
            };
            return LayoutInfo {
                size: rounded,
                align: max_align,
                members: new_members,
            };
        }

        let mut bitfield_base_offset: Option<u64> = None;
        let mut bitfield_bits_used: u32 = 0;
        let mut bitfield_type_size: u64 = 0;

        for member in members {
            let m_align = self.align_of(member.type_id);
            let m_size = self.size_of(member.type_id);

            if m_align > max_align {
                max_align = m_align;
            }

            if let (Some(bit_off), Some(bit_width)) = (member.bit_offset, member.bit_width) {
                if bit_off == 0 && bit_width > 0 {
                    if bitfield_base_offset.is_some() {
                        let bits_remaining = (bitfield_type_size * 8) as u32 - bitfield_bits_used;
                        if bits_remaining < bit_width {
                            offset = bitfield_base_offset.unwrap() + bitfield_type_size;
                            bitfield_base_offset = None;
                            bitfield_bits_used = 0;
                        }
                    }
                    if bitfield_base_offset.is_none() {
                        offset = (offset + m_align - 1) / m_align * m_align;
                        bitfield_base_offset = Some(offset);
                        bitfield_type_size = m_size;
                        bitfield_bits_used = 0;
                    }
                    new_members.push(StructMember {
                        name: member.name.clone(),
                        type_id: member.type_id,
                        offset: bitfield_base_offset.unwrap(),
                        bit_offset: Some(bitfield_bits_used),
                        bit_width: Some(bit_width),
                    });
                    bitfield_bits_used += bit_width;
                } else {
                    if bitfield_base_offset.is_some() {
                        offset = bitfield_base_offset.unwrap() + bitfield_type_size;
                        bitfield_base_offset = None;
                        bitfield_bits_used = 0;
                    }
                    offset = (offset + m_align - 1) / m_align * m_align;
                    new_members.push(StructMember {
                        name: member.name.clone(),
                        type_id: member.type_id,
                        offset,
                        bit_offset: member.bit_offset,
                        bit_width: member.bit_width,
                    });
                    offset += m_size;
                }
            } else {
                if bitfield_base_offset.is_some() {
                    offset = bitfield_base_offset.unwrap() + bitfield_type_size;
                    bitfield_base_offset = None;
                    bitfield_bits_used = 0;
                }
                offset = (offset + m_align - 1) / m_align * m_align;
                new_members.push(StructMember {
                    name: member.name.clone(),
                    type_id: member.type_id,
                    offset,
                    bit_offset: None,
                    bit_width: None,
                });
                offset += m_size;
            }
        }

        if bitfield_base_offset.is_some() {
            offset = bitfield_base_offset.unwrap() + bitfield_type_size;
        }

        let rounded = if max_align > 0 {
            (offset + max_align - 1) / max_align * max_align
        } else {
            offset
        };

        LayoutInfo {
            size: rounded,
            align: max_align,
            members: new_members,
        }
    }

    fn update_struct_layout(
        &mut self,
        id: TypeId,
        size: u64,
        align: u64,
        members: Vec<StructMember>,
    ) {
        if let Some(ty) = self.types.get_mut(id.0 as usize) {
            match ty {
                CType::Struct {
                    size: s,
                    align: a,
                    members: m,
                    ..
                } => {
                    *s = size;
                    *a = align;
                    *m = members;
                }
                CType::Union {
                    size: s,
                    align: a,
                    members: m,
                    ..
                } => {
                    *s = size;
                    *a = align;
                    *m = members;
                }
                _ => {}
            }
        }
    }

    pub fn integer_rank(&self, id: TypeId) -> u32 {
        let id = self.resolve_typedef(id);
        match self.get_type(id) {
            Some(CType::Bool) => 0,
            Some(CType::Char { .. }) => 1,
            Some(CType::Short { .. }) => 2,
            Some(CType::Int { .. }) => 3,
            Some(CType::Long { .. }) => 4,
            Some(CType::LongLong { .. }) => 5,
            Some(CType::Float) => 6,
            Some(CType::Double) => 7,
            Some(CType::LongDouble) => 8,
            _ => 3,
        }
    }

    pub fn is_signed(&self, id: TypeId) -> bool {
        let id = self.resolve_typedef(id);
        match self.get_type(id) {
            Some(CType::Bool) => false,
            Some(CType::Char { signed }) => *signed,
            Some(CType::Short { signed }) => *signed,
            Some(CType::Int { signed }) => *signed,
            Some(CType::Long { signed }) => *signed,
            Some(CType::LongLong { signed }) => *signed,
            _ => false,
        }
    }

    pub fn pointer_base(&self, id: TypeId) -> Option<TypeId> {
        match self.get_type(id) {
            Some(CType::Pointer { base }) => Some(*base),
            _ => None,
        }
    }

    pub fn void_type_id() -> TypeId {
        TypeId::VOID
    }

    pub fn bool_type_id() -> TypeId {
        TypeId::BOOL
    }

    pub fn char_type_id() -> TypeId {
        TypeId::CHAR
    }

    pub fn int_type_id() -> TypeId {
        TypeId::INT
    }

    pub fn long_type_id() -> TypeId {
        TypeId::LONG
    }

    pub fn double_type_id() -> TypeId {
        TypeId::DOUBLE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_id_constants() {
        assert_eq!(TypeId::VOID, TypeId(0));
        assert_eq!(TypeId::BOOL, TypeId(1));
        assert_eq!(TypeId::CHAR, TypeId(2));
        assert_eq!(TypeId::INT, TypeId(7));
        assert_eq!(TypeId::DOUBLE, TypeId(14));
        assert_eq!(TypeId::LONGDOUBLE, TypeId(15));
    }

    #[test]
    fn test_type_system_new_has_primitives() {
        let ts = TypeSystem::new();
        assert!(matches!(ts.get_type(TypeId::VOID), Some(CType::Void)));
        assert!(matches!(ts.get_type(TypeId::BOOL), Some(CType::Bool)));
        assert!(matches!(
            ts.get_type(TypeId::CHAR),
            Some(CType::Char { signed: false })
        ));
        assert!(matches!(
            ts.get_type(TypeId::INT),
            Some(CType::Int { signed: true })
        ));
        assert!(matches!(ts.get_type(TypeId::FLOAT), Some(CType::Float)));
        assert!(matches!(ts.get_type(TypeId::DOUBLE), Some(CType::Double)));
    }

    #[test]
    fn test_size_of_primitives() {
        let ts = TypeSystem::new();
        assert_eq!(ts.size_of(TypeId::VOID), 0);
        assert_eq!(ts.size_of(TypeId::BOOL), 1);
        assert_eq!(ts.size_of(TypeId::CHAR), 1);
        assert_eq!(ts.size_of(TypeId::SHORT), 2);
        assert_eq!(ts.size_of(TypeId::INT), 4);
        assert_eq!(ts.size_of(TypeId::LONG), 8);
        assert_eq!(ts.size_of(TypeId::LONGLONG), 8);
        assert_eq!(ts.size_of(TypeId::FLOAT), 4);
        assert_eq!(ts.size_of(TypeId::DOUBLE), 8);
        assert_eq!(ts.size_of(TypeId::LONGDOUBLE), 16);
    }

    #[test]
    fn test_align_of_primitives() {
        let ts = TypeSystem::new();
        assert_eq!(ts.align_of(TypeId::VOID), 1);
        assert_eq!(ts.align_of(TypeId::BOOL), 1);
        assert_eq!(ts.align_of(TypeId::CHAR), 1);
        assert_eq!(ts.align_of(TypeId::SHORT), 2);
        assert_eq!(ts.align_of(TypeId::INT), 4);
        assert_eq!(ts.align_of(TypeId::LONG), 8);
        assert_eq!(ts.align_of(TypeId::LONGLONG), 8);
        assert_eq!(ts.align_of(TypeId::FLOAT), 4);
        assert_eq!(ts.align_of(TypeId::DOUBLE), 8);
        assert_eq!(ts.align_of(TypeId::LONGDOUBLE), 16);
    }

    #[test]
    fn test_pointer_type_creation() {
        let mut ts = TypeSystem::new();
        let int_ptr = ts.add_type(CType::Pointer { base: TypeId::INT });
        assert!(ts.is_pointer(int_ptr));
        assert_eq!(ts.size_of(int_ptr), 8);
        assert_eq!(ts.align_of(int_ptr), 8);
        assert_eq!(ts.pointer_base(int_ptr), Some(TypeId::INT));
    }

    #[test]
    fn test_pointer_to_pointer() {
        let mut ts = TypeSystem::new();
        let int_ptr = ts.add_type(CType::Pointer { base: TypeId::INT });
        let ptr_ptr = ts.add_type(CType::Pointer { base: int_ptr });
        assert!(ts.is_pointer(ptr_ptr));
        assert_eq!(ts.pointer_base(ptr_ptr), Some(int_ptr));
        assert_eq!(ts.size_of(ptr_ptr), 8);
    }

    #[test]
    fn test_array_type_creation() {
        let mut ts = TypeSystem::new();
        let arr = ts.add_type(CType::Array {
            element: TypeId::INT,
            size: Some(10),
        });
        assert_eq!(ts.size_of(arr), 40);
        assert_eq!(ts.align_of(arr), 4);
    }

    #[test]
    fn test_array_incomplete_size() {
        let mut ts = TypeSystem::new();
        let arr = ts.add_type(CType::Array {
            element: TypeId::INT,
            size: None,
        });
        assert_eq!(ts.size_of(arr), 0);
    }

    #[test]
    fn test_struct_layout_computation() {
        let mut ts = TypeSystem::new();
        let member1 = StructMember {
            name: "a".to_string(),
            type_id: TypeId::CHAR,
            offset: 0,
            bit_offset: None,
            bit_width: None,
        };
        let member2 = StructMember {
            name: "b".to_string(),
            type_id: TypeId::INT,
            offset: 0,
            bit_offset: None,
            bit_width: None,
        };
        let struct_id = ts.add_type(CType::Struct {
            name: Some("Test".to_string()),
            members: vec![member1, member2],
            size: 0,
            align: 0,
        });
        ts.compute_struct_layout(struct_id);
        assert_eq!(ts.size_of(struct_id), 8);
        assert_eq!(ts.align_of(struct_id), 4);
    }

    #[test]
    fn test_struct_padding() {
        let mut ts = TypeSystem::new();
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
                type_id: TypeId::SHORT,
                offset: 0,
                bit_offset: None,
                bit_width: None,
            },
            StructMember {
                name: "c".to_string(),
                type_id: TypeId::INT,
                offset: 0,
                bit_offset: None,
                bit_width: None,
            },
        ];
        let struct_id = ts.add_type(CType::Struct {
            name: Some("Padded".to_string()),
            members,
            size: 0,
            align: 0,
        });
        ts.compute_struct_layout(struct_id);
        assert_eq!(ts.size_of(struct_id), 8);
    }

    #[test]
    fn test_union_layout() {
        let mut ts = TypeSystem::new();
        let members = vec![
            StructMember {
                name: "a".to_string(),
                type_id: TypeId::INT,
                offset: 0,
                bit_offset: None,
                bit_width: None,
            },
            StructMember {
                name: "b".to_string(),
                type_id: TypeId::DOUBLE,
                offset: 0,
                bit_offset: None,
                bit_width: None,
            },
        ];
        let union_id = ts.add_type(CType::Union {
            name: Some("Data".to_string()),
            members,
            size: 0,
            align: 0,
        });
        ts.compute_struct_layout(union_id);
        assert_eq!(ts.size_of(union_id), 8);
        assert_eq!(ts.align_of(union_id), 8);
    }

    #[test]
    fn test_typedef_chain_resolution() {
        let mut ts = TypeSystem::new();
        let my_int = ts.add_type(CType::Typedef {
            name: "my_int".to_string(),
            underlying: TypeId::INT,
        });
        let my_int2 = ts.add_type(CType::Typedef {
            name: "my_int2".to_string(),
            underlying: my_int,
        });
        let resolved = ts.resolve_typedef(my_int2);
        assert_eq!(resolved, TypeId::INT);
    }

    #[test]
    fn test_type_qualifiers() {
        let mut ts = TypeSystem::new();
        let const_int = ts.add_type(CType::Qualified {
            qualifiers: TypeQualifiers::CONST,
            base: TypeId::INT,
        });
        assert!(!ts.is_pointer(const_int));
        assert!(ts.is_integer(const_int));
        let stripped = ts.strip_qualifiers(const_int);
        assert_eq!(stripped, TypeId::INT);
    }

    #[test]
    fn test_combined_qualifiers() {
        let mut ts = TypeSystem::new();
        let cv_int = ts.add_type(CType::Qualified {
            qualifiers: TypeQualifiers::CONST | TypeQualifiers::VOLATILE,
            base: TypeId::INT,
        });
        let stripped = ts.strip_qualifiers(cv_int);
        assert_eq!(stripped, TypeId::INT);
        assert!(ts.is_integer(cv_int));
    }

    #[test]
    fn test_function_type_creation() {
        let mut ts = TypeSystem::new();
        let func = ts.add_type(CType::Function {
            return_type: TypeId::INT,
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
        assert!(matches!(ts.get_type(func), Some(CType::Function { .. })));
    }

    #[test]
    fn test_variadic_function() {
        let mut ts = TypeSystem::new();
        let func = ts.add_type(CType::Function {
            return_type: TypeId::INT,
            params: vec![ParamType {
                name: Some("fmt".to_string()),
                type_id: TypeId::CHAR,
            }],
            variadic: true,
        });
        if let Some(CType::Function { variadic, .. }) = ts.get_type(func) {
            assert!(*variadic);
        } else {
            panic!("Expected function type");
        }
    }

    #[test]
    fn test_enum_type() {
        let mut ts = TypeSystem::new();
        let enum_id = ts.add_type(CType::Enum {
            name: Some("Color".to_string()),
            underlying: TypeId::INT,
        });
        assert_eq!(ts.size_of(enum_id), 4);
        assert_eq!(ts.align_of(enum_id), 4);
        assert!(ts.is_integer(enum_id));
    }

    #[test]
    fn test_is_integer() {
        let ts = TypeSystem::new();
        assert!(ts.is_integer(TypeId::BOOL));
        assert!(ts.is_integer(TypeId::CHAR));
        assert!(ts.is_integer(TypeId::INT));
        assert!(ts.is_integer(TypeId::LONG));
        assert!(!ts.is_integer(TypeId::VOID));
        assert!(!ts.is_integer(TypeId::FLOAT));
    }

    #[test]
    fn test_is_floating() {
        let ts = TypeSystem::new();
        assert!(ts.is_floating(TypeId::FLOAT));
        assert!(ts.is_floating(TypeId::DOUBLE));
        assert!(ts.is_floating(TypeId::LONGDOUBLE));
        assert!(!ts.is_floating(TypeId::INT));
    }

    #[test]
    fn test_is_arithmetic() {
        let ts = TypeSystem::new();
        assert!(ts.is_arithmetic(TypeId::INT));
        assert!(ts.is_arithmetic(TypeId::FLOAT));
        assert!(!ts.is_arithmetic(TypeId::VOID));
    }

    #[test]
    fn test_is_scalar() {
        let mut ts = TypeSystem::new();
        assert!(ts.is_scalar(TypeId::INT));
        assert!(ts.is_scalar(TypeId::DOUBLE));
        let ptr = ts.add_type(CType::Pointer { base: TypeId::INT });
        assert!(ts.is_scalar(ptr));
        assert!(!ts.is_scalar(TypeId::VOID));
    }

    #[test]
    fn test_bitfield_layout() {
        let mut ts = TypeSystem::new();
        let members = vec![
            StructMember {
                name: "a".to_string(),
                type_id: TypeId::INT,
                offset: 0,
                bit_offset: Some(0),
                bit_width: Some(1),
            },
            StructMember {
                name: "b".to_string(),
                type_id: TypeId::INT,
                offset: 0,
                bit_offset: Some(0),
                bit_width: Some(2),
            },
            StructMember {
                name: "c".to_string(),
                type_id: TypeId::INT,
                offset: 0,
                bit_offset: Some(0),
                bit_width: Some(5),
            },
        ];
        let struct_id = ts.add_type(CType::Struct {
            name: Some("Bits".to_string()),
            members,
            size: 0,
            align: 0,
        });
        ts.compute_struct_layout(struct_id);
        assert_eq!(ts.size_of(struct_id), 4);
    }

    #[test]
    fn test_struct_with_flexible_array_member() {
        let mut ts = TypeSystem::new();
        let flex = ts.add_type(CType::Array {
            element: TypeId::CHAR,
            size: None,
        });
        let struct_id = ts.add_type(CType::Struct {
            name: Some("Flex".to_string()),
            members: vec![
                StructMember {
                    name: "len".to_string(),
                    type_id: TypeId::INT,
                    offset: 0,
                    bit_offset: None,
                    bit_width: None,
                },
                StructMember {
                    name: "data".to_string(),
                    type_id: flex,
                    offset: 0,
                    bit_offset: None,
                    bit_width: None,
                },
            ],
            size: 0,
            align: 0,
        });
        ts.compute_struct_layout(struct_id);
        assert_eq!(ts.size_of(struct_id), 4);
        assert_eq!(ts.align_of(struct_id), 4);
    }

    #[test]
    fn test_struct_with_nested_types() {
        let mut ts = TypeSystem::new();
        let inner = ts.add_type(CType::Struct {
            name: Some("Inner".to_string()),
            members: vec![StructMember {
                name: "x".to_string(),
                type_id: TypeId::INT,
                offset: 0,
                bit_offset: None,
                bit_width: None,
            }],
            size: 0,
            align: 0,
        });
        ts.compute_struct_layout(inner);
        let outer = ts.add_type(CType::Struct {
            name: Some("Outer".to_string()),
            members: vec![
                StructMember {
                    name: "inner".to_string(),
                    type_id: inner,
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
            ],
            size: 0,
            align: 0,
        });
        ts.compute_struct_layout(outer);
        assert_eq!(ts.size_of(outer), 16);
    }

    #[test]
    fn test_type_deduplication() {
        let mut ts = TypeSystem::new();
        let ptr1 = ts.add_type(CType::Pointer { base: TypeId::INT });
        let ptr2 = ts.add_type(CType::Pointer { base: TypeId::INT });
        assert_eq!(ptr1, ptr2);
    }

    #[test]
    fn test_unsigned_types() {
        let ts = TypeSystem::new();
        assert!(matches!(
            ts.get_type(TypeId::UINT),
            Some(CType::Int { signed: false })
        ));
        assert!(matches!(
            ts.get_type(TypeId::ULONG),
            Some(CType::Long { signed: false })
        ));
        assert!(matches!(
            ts.get_type(TypeId::ULONGLONG),
            Some(CType::LongLong { signed: false })
        ));
    }

    #[test]
    fn test_integer_rank() {
        let ts = TypeSystem::new();
        assert!(ts.integer_rank(TypeId::CHAR) < ts.integer_rank(TypeId::SHORT));
        assert!(ts.integer_rank(TypeId::SHORT) < ts.integer_rank(TypeId::INT));
        assert!(ts.integer_rank(TypeId::INT) < ts.integer_rank(TypeId::LONG));
        assert!(ts.integer_rank(TypeId::LONG) < ts.integer_rank(TypeId::LONGLONG));
        assert!(ts.integer_rank(TypeId::INT) < ts.integer_rank(TypeId::FLOAT));
        assert!(ts.integer_rank(TypeId::FLOAT) < ts.integer_rank(TypeId::DOUBLE));
    }
}

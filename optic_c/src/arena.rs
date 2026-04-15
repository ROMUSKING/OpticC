pub struct NodeOffset(pub u32);
#[repr(C)] pub struct CAstNode { pub kind: u16, }
pub struct Arena {}
impl Arena { pub fn allocate(&self) {} }

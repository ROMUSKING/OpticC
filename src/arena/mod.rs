use memmap2::MmapMut;
use std::fs::OpenOptions;
use std::path::Path;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeOffset(pub u32);

bitflags::bitflags! {
    pub struct NodeFlags: u16 {
        const IS_CONST    = 0b0000_0001;
        const IS_VOLATILE = 0b0000_0010;
        const IS_RESTRICT = 0b0000_0100;
        const IS_MACRO    = 0b0000_1000;
    }
}

#[repr(C)]
pub struct CAstNode {
    pub kind: u16,
    pub flags: NodeFlags,
    pub left_child: NodeOffset,
    pub next_sibling: NodeOffset,
    pub data_offset: u32, // Offset into string interner
}

pub struct Arena {
    mmap: MmapMut,
    len: usize,
}

impl Arena {
    pub fn new<P: AsRef<Path>>(path: P, capacity: usize) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .read(true).write(true).create(true)
            .open(path)?;
        
        file.set_len(capacity as u64)?;
        let mmap = unsafe { MmapMut::map_mut(&file)? };
        
        Ok(Self { mmap, len: 0 })
    }

    #[inline(always)]
    pub fn alloc(&mut self, node: CAstNode) -> NodeOffset {
        let offset = self.len;
        let node_size = std::mem::size_of::<CAstNode>();

        if offset + node_size > self.mmap.len() {
            panic!("Arena capacity exceeded: cannot allocate {} bytes at offset {}", node_size, offset);
        }
        
        unsafe {
            let ptr = self.mmap.as_mut_ptr().add(offset);
            std::ptr::write(ptr as *mut CAstNode, node);
        }
        
        self.len += node_size;
        NodeOffset(offset as u32)
    }
    
    #[inline(always)]
    pub fn get(&self, offset: NodeOffset) -> &CAstNode {
        let offset = offset.0 as usize;
        let node_size = std::mem::size_of::<CAstNode>();

        if offset + node_size > self.mmap.len() {
            panic!("Arena access out of bounds: offset {} with node size {}", offset, node_size);
        }

        unsafe {
            let ptr = self.mmap.as_ptr().add(offset);
            &*(ptr as *const CAstNode)
        }
    }
}

use memmap2::MmapMut;
use std::fs::OpenOptions;
use std::path::Path;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeOffset(pub u32);

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_arena_alloc_and_get() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        let mut arena = Arena::new(path, 1024).unwrap();

        let node1 = CAstNode {
            kind: 1,
            flags: NodeFlags::IS_CONST,
            left_child: NodeOffset(0),
            next_sibling: NodeOffset(0),
            data_offset: 10,
        };

        let offset1 = arena.alloc(node1);
        assert_eq!(offset1, NodeOffset(0));

        let node2 = CAstNode {
            kind: 2,
            flags: NodeFlags::IS_VOLATILE,
            left_child: NodeOffset(1),
            next_sibling: NodeOffset(2),
            data_offset: 20,
        };

        let offset2 = arena.alloc(node2);
        assert_eq!(offset2.0 as usize, std::mem::size_of::<CAstNode>());

        let retrieved_node1 = arena.get(offset1);
        assert_eq!(retrieved_node1.kind, 1);
        assert_eq!(retrieved_node1.flags, NodeFlags::IS_CONST);
        assert_eq!(retrieved_node1.data_offset, 10);

        let retrieved_node2 = arena.get(offset2);
        assert_eq!(retrieved_node2.kind, 2);
        assert_eq!(retrieved_node2.flags, NodeFlags::IS_VOLATILE);
        assert_eq!(retrieved_node2.data_offset, 20);
    }
}

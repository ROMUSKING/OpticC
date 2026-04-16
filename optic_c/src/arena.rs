use bitflags::bitflags;
use memmap2::MmapMut;
use std::fs::OpenOptions;
use std::io;
use std::path::Path;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeOffset(pub u32);

impl NodeOffset {
    pub const NULL: NodeOffset = NodeOffset(0);
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NodeFlags: u16 {
        const NONE = 0;
        const IS_VALID = 1 << 0;
        const HAS_ERROR = 1 << 1;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CAstNode {
    pub kind: u16,
    pub flags: NodeFlags,
    pub parent: NodeOffset,
    pub first_child: NodeOffset,
    pub next_sibling: NodeOffset,
    pub data: u32,
}

pub struct Arena {
    mmap: MmapMut,
    capacity: u32,
    allocated: u32,
    string_start: u32,
    string_allocated: u32,
}

impl Arena {
    pub fn new<P: AsRef<Path>>(path: P, capacity: u32) -> io::Result<Self> {
        if capacity == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Capacity must be greater than 0",
            ));
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        let node_capacity = capacity / 2;
        let string_start = node_capacity + 1;

        let byte_capacity = capacity as u64 * std::mem::size_of::<CAstNode>() as u64;
        file.set_len(byte_capacity)?;

        let mmap = unsafe { MmapMut::map_mut(&file)? };

        Ok(Arena {
            mmap,
            capacity,
            allocated: 1,
            string_start,
            string_allocated: 0,
        })
    }

    pub fn alloc(&mut self, node: CAstNode) -> Option<NodeOffset> {
        if self.allocated >= self.string_start {
            return None;
        }

        let offset = NodeOffset(self.allocated);
        self.allocated += 1;

        let byte_offset = (offset.0 as usize) * std::mem::size_of::<CAstNode>();

        unsafe {
            let ptr = self.mmap.as_mut_ptr().add(byte_offset) as *mut CAstNode;
            std::ptr::write(ptr, node);
        }

        Some(offset)
    }

    pub fn get(&self, offset: NodeOffset) -> Option<&CAstNode> {
        if offset.0 == 0 || offset.0 >= self.allocated {
            return None;
        }

        if offset.0 >= self.string_start {
            return None;
        }

        let byte_offset = (offset.0 as usize) * std::mem::size_of::<CAstNode>();
        let node = unsafe { &*(self.mmap.as_ptr().add(byte_offset) as *const CAstNode) };
        Some(node)
    }

    pub fn allocated(&self) -> u32 {
        self.allocated
    }

    pub fn get_mut(&mut self, offset: NodeOffset) -> Option<&mut CAstNode> {
        if offset.0 == 0 || offset.0 >= self.allocated {
            return None;
        }

        if offset.0 >= self.string_start {
            return None;
        }

        let byte_offset = (offset.0 as usize) * std::mem::size_of::<CAstNode>();
        let node = unsafe { &mut *(self.mmap.as_mut_ptr().add(byte_offset) as *mut CAstNode) };
        Some(node)
    }

    pub fn store_string(&mut self, s: &str) -> Option<NodeOffset> {
        let slot_size = std::mem::size_of::<CAstNode>() as u32;
        let len = s.len() as u32;
        let total_bytes = std::mem::size_of::<u32>() + s.len();
        let slots_needed = (total_bytes as u32 + slot_size - 1) / slot_size;

        let string_allocated_before = self.string_allocated;
        self.string_allocated += slots_needed;

        if self.string_start + self.string_allocated > self.capacity {
            self.string_allocated = string_allocated_before;
            return None;
        }

        let string_byte_offset = (self.string_start as usize) * std::mem::size_of::<CAstNode>()
            + (string_allocated_before as usize) * std::mem::size_of::<CAstNode>();

        unsafe {
            let ptr = self.mmap.as_mut_ptr().add(string_byte_offset);
            std::ptr::write(ptr as *mut u32, len);
            std::ptr::copy_nonoverlapping(
                s.as_bytes().as_ptr(),
                ptr.add(std::mem::size_of::<u32>()),
                s.len()
            );
        }

        let offset = self.string_start + string_allocated_before;
        Some(NodeOffset(offset))
    }

    pub fn get_string(&self, offset: NodeOffset) -> Option<String> {
        if offset.0 < self.string_start || offset.0 >= self.string_start + self.string_allocated {
            return None;
        }

        let slot_size = std::mem::size_of::<CAstNode>() as u32;
        let string_slot = offset.0 - self.string_start;
        let string_byte_offset = (self.string_start as usize) * std::mem::size_of::<CAstNode>()
            + (string_slot as usize) * std::mem::size_of::<CAstNode>();

        unsafe {
            let len = std::ptr::read(self.mmap.as_ptr().add(string_byte_offset) as *const u32);
            let bytes = std::slice::from_raw_parts(
                self.mmap.as_ptr().add(string_byte_offset + std::mem::size_of::<u32>()),
                len as usize
            );
            String::from_utf8(bytes.to_vec()).ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::time::Instant;

    #[test]
    fn test_high_speed_sequential_allocation() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        let num_allocations = 10_000;

        let mut arena = Arena::new(path, num_allocations * 2 + 2).unwrap();

        let dummy_node = CAstNode {
            kind: 0,
            flags: NodeFlags::NONE,
            parent: NodeOffset::NULL,
            first_child: NodeOffset::NULL,
            next_sibling: NodeOffset::NULL,
            data: 0,
        };

        let start_time = Instant::now();

        for _ in 0..num_allocations {
            let offset = arena.alloc(dummy_node).unwrap();
            assert!(offset.0 > 0);
        }

        let elapsed = start_time.elapsed();
        println!("Allocated {} nodes in {:?}", num_allocations, elapsed);

        // Quick verification of the last node
        let last_offset = NodeOffset(num_allocations);
        let node = arena.get(last_offset).unwrap();
        assert_eq!(node.kind, dummy_node.kind);
    }
}

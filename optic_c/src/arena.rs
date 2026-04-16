use memmap2::MmapMut;
use std::fs::OpenOptions;
use std::io;
use std::path::Path;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeOffset(pub u32);

impl NodeOffset {
    pub const NULL: NodeOffset = NodeOffset(0);

    #[inline]
    pub fn is_null(&self) -> bool {
        self.0 == 0
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    pub file_id: u32,
    pub line: u32,
    pub column: u32,
    pub length: u32,
}

impl SourceLocation {
    pub const fn unknown() -> Self {
        SourceLocation {
            file_id: 0,
            line: 0,
            column: 0,
            length: 0,
        }
    }
}

bitflags::bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NodeFlags: u16 {
        const NONE = 0;
        const IS_VALID = 1 << 0;
        const HAS_ERROR = 1 << 1;
        const IS_SYNTHETIC = 1 << 2;
        const HAS_PAYLOAD = 1 << 3;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CAstNode {
    pub kind: u16,
    pub flags: NodeFlags,
    pub parent: NodeOffset,
    pub first_child: NodeOffset,
    pub last_child: NodeOffset,
    pub next_sibling: NodeOffset,
    pub prev_sibling: NodeOffset,
    pub child_count: u16,
    pub data: u32,
    pub source: SourceLocation,
    pub payload_offset: NodeOffset,
    pub payload_len: u32,
}

impl CAstNode {
    pub const fn new(kind: u16) -> Self {
        CAstNode {
            kind,
            flags: NodeFlags::NONE,
            parent: NodeOffset::NULL,
            first_child: NodeOffset::NULL,
            last_child: NodeOffset::NULL,
            next_sibling: NodeOffset::NULL,
            prev_sibling: NodeOffset::NULL,
            child_count: 0,
            data: 0,
            source: SourceLocation::unknown(),
            payload_offset: NodeOffset::NULL,
            payload_len: 0,
        }
    }

    pub const fn with_source(mut self, source: SourceLocation) -> Self {
        self.source = source;
        self
    }

    pub const fn with_flags(mut self, flags: NodeFlags) -> Self {
        self.flags = flags;
        self
    }
}

#[derive(Debug)]
pub enum ArenaError {
    ZeroCapacity,
    AllocationFull,
    StringPoolFull,
    InvalidOffset,
    IoError(io::Error),
}

impl std::fmt::Display for ArenaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArenaError::ZeroCapacity => write!(f, "arena capacity must be greater than zero"),
            ArenaError::AllocationFull => write!(f, "arena node allocation pool is full"),
            ArenaError::StringPoolFull => write!(f, "arena string pool is full"),
            ArenaError::InvalidOffset => write!(f, "offset is out of bounds or invalid"),
            ArenaError::IoError(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for ArenaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ArenaError::IoError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for ArenaError {
    fn from(err: io::Error) -> Self {
        ArenaError::IoError(err)
    }
}

pub struct Arena {
    mmap: MmapMut,
    node_capacity: u32,
    node_allocated: u32,
    string_start: u32,
    string_capacity: u32,
    string_allocated: u32,
}

impl Arena {
    pub fn new<P: AsRef<Path>>(path: P, node_capacity: u32) -> Result<Self, ArenaError> {
        if node_capacity == 0 {
            return Err(ArenaError::ZeroCapacity);
        }

        let string_capacity = node_capacity / 4;
        let total_slots = node_capacity + string_capacity;
        let byte_size = (total_slots as u64) * (std::mem::size_of::<CAstNode>() as u64);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        file.set_len(byte_size)?;

        let mmap = unsafe { MmapMut::map_mut(&file)? };

        Ok(Arena {
            mmap,
            node_capacity,
            node_allocated: 1,
            string_start: node_capacity + 1,
            string_capacity,
            string_allocated: 0,
        })
    }

    #[inline]
    pub fn alloc(&mut self, node: CAstNode) -> Option<NodeOffset> {
        if self.node_allocated >= self.node_capacity {
            return None;
        }

        let offset = NodeOffset(self.node_allocated);
        let byte_offset = (self.node_allocated as usize) * std::mem::size_of::<CAstNode>();
        self.node_allocated += 1;

        unsafe {
            let ptr = self.mmap.as_mut_ptr().add(byte_offset) as *mut CAstNode;
            std::ptr::write(ptr, node);
        }

        Some(offset)
    }

    #[inline]
    pub fn get(&self, offset: NodeOffset) -> Option<&CAstNode> {
        if offset.0 == 0 || offset.0 >= self.node_allocated {
            return None;
        }

        let byte_offset = (offset.0 as usize) * std::mem::size_of::<CAstNode>();
        unsafe {
            let ptr = self.mmap.as_ptr().add(byte_offset) as *const CAstNode;
            Some(&*ptr)
        }
    }

    #[inline]
    pub fn get_mut(&mut self, offset: NodeOffset) -> Option<&mut CAstNode> {
        if offset.0 == 0 || offset.0 >= self.node_allocated {
            return None;
        }

        let byte_offset = (offset.0 as usize) * std::mem::size_of::<CAstNode>();
        unsafe {
            let ptr = self.mmap.as_mut_ptr().add(byte_offset) as *mut CAstNode;
            Some(&mut *ptr)
        }
    }

    pub fn store_string(&mut self, s: &str) -> Option<NodeOffset> {
        if s.is_empty() {
            return Some(NodeOffset::NULL);
        }

        let slot_size = std::mem::size_of::<CAstNode>() as u32;
        let total_bytes = std::mem::size_of::<u32>() + s.len();
        let slots_needed = (total_bytes as u32 + slot_size - 1) / slot_size;

        if self.string_allocated + slots_needed > self.string_capacity {
            return None;
        }

        let slot_index = self.string_allocated;
        self.string_allocated += slots_needed;

        let byte_offset = (self.string_start as usize) * std::mem::size_of::<CAstNode>()
            + (slot_index as usize) * std::mem::size_of::<CAstNode>();

        unsafe {
            let ptr = self.mmap.as_mut_ptr().add(byte_offset);
            std::ptr::write(ptr as *mut u32, s.len() as u32);
            std::ptr::copy_nonoverlapping(
                s.as_bytes().as_ptr(),
                ptr.add(std::mem::size_of::<u32>()),
                s.len(),
            );
        }

        Some(NodeOffset(self.string_start + slot_index))
    }

    pub fn get_string(&self, offset: NodeOffset) -> Option<&str> {
        if offset.is_null() {
            return Some("");
        }

        if offset.0 < self.string_start
            || offset.0 >= self.string_start + self.string_allocated
        {
            return None;
        }

        let slot_size = std::mem::size_of::<CAstNode>() as u32;
        let string_slot = offset.0 - self.string_start;
        let byte_offset = (self.string_start as usize) * std::mem::size_of::<CAstNode>()
            + (string_slot as usize) * std::mem::size_of::<CAstNode>();

        unsafe {
            let len_ptr = self.mmap.as_ptr().add(byte_offset) as *const u32;
            let len = std::ptr::read(len_ptr) as usize;
            let bytes = std::slice::from_raw_parts(
                self.mmap.as_ptr().add(byte_offset + std::mem::size_of::<u32>()),
                len,
            );
            std::str::from_utf8(bytes).ok()
        }
    }

    pub fn store_payload(&mut self, data: &[u8]) -> Option<(NodeOffset, u32)> {
        if data.is_empty() {
            return Some((NodeOffset::NULL, 0));
        }

        let slot_size = std::mem::size_of::<CAstNode>() as u32;
        let total_bytes = std::mem::size_of::<u32>() + data.len();
        let slots_needed = (total_bytes as u32 + slot_size - 1) / slot_size;

        if self.string_allocated + slots_needed > self.string_capacity {
            return None;
        }

        let slot_index = self.string_allocated;
        self.string_allocated += slots_needed;

        let byte_offset = (self.string_start as usize) * std::mem::size_of::<CAstNode>()
            + (slot_index as usize) * std::mem::size_of::<CAstNode>();

        unsafe {
            let ptr = self.mmap.as_mut_ptr().add(byte_offset);
            std::ptr::write(ptr as *mut u32, data.len() as u32);
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                ptr.add(std::mem::size_of::<u32>()),
                data.len(),
            );
        }

        let offset = self.string_start + slot_index;
        Some((NodeOffset(offset), data.len() as u32))
    }

    pub fn get_payload(&self, offset: NodeOffset, len: u32) -> Option<&[u8]> {
        if offset.is_null() {
            return Some(&[]);
        }

        if offset.0 < self.string_start
            || offset.0 >= self.string_start + self.string_allocated
        {
            return None;
        }

        let slot_size = std::mem::size_of::<CAstNode>() as u32;
        let string_slot = offset.0 - self.string_start;
        let byte_offset = (self.string_start as usize) * std::mem::size_of::<CAstNode>()
            + (string_slot as usize) * std::mem::size_of::<CAstNode>();

        unsafe {
            let stored_len = std::ptr::read(
                self.mmap.as_ptr().add(byte_offset) as *const u32,
            ) as usize;

            if stored_len != len as usize {
                return None;
            }

            Some(std::slice::from_raw_parts(
                self.mmap.as_ptr().add(byte_offset + std::mem::size_of::<u32>()),
                len as usize,
            ))
        }
    }

    #[inline]
    pub fn node_capacity(&self) -> u32 {
        self.node_capacity
    }

    #[inline]
    pub fn nodes_allocated(&self) -> u32 {
        self.node_allocated.saturating_sub(1)
    }

    #[inline]
    pub fn remaining_nodes(&self) -> u32 {
        self.node_capacity.saturating_sub(self.node_allocated)
    }

    #[inline]
    pub fn string_capacity(&self) -> u32 {
        self.string_capacity
    }

    #[inline]
    pub fn string_bytes_used(&self) -> u32 {
        self.string_allocated * std::mem::size_of::<CAstNode>() as u32
    }

    pub fn flush(&self) -> io::Result<()> {
        self.mmap.flush()
    }

    pub fn link_child(parent_offset: NodeOffset, child_offset: NodeOffset) -> CAstNode {
        CAstNode {
            parent: parent_offset,
            first_child: child_offset,
            last_child: child_offset,
            child_count: 1,
            ..CAstNode::new(0)
        }
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::time::Instant;

    #[test]
    fn test_basic_allocation() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut arena = Arena::new(temp_file.path(), 1024).unwrap();

        let node = CAstNode::new(1)
            .with_source(SourceLocation {
                file_id: 1,
                line: 10,
                column: 5,
                length: 20,
            })
            .with_flags(NodeFlags::IS_VALID);

        let offset = arena.alloc(node).unwrap();
        assert!(!offset.is_null());

        let retrieved = arena.get(offset).unwrap();
        assert_eq!(retrieved.kind, 1);
        assert_eq!(retrieved.source.line, 10);
        assert_eq!(retrieved.source.column, 5);
        assert!(retrieved.flags.contains(NodeFlags::IS_VALID));
    }

    #[test]
    fn test_null_offset() {
        let temp_file = NamedTempFile::new().unwrap();
        let arena = Arena::new(temp_file.path(), 1024).unwrap();

        assert!(NodeOffset::NULL.is_null());
        assert_eq!(arena.get(NodeOffset::NULL), None);
    }

    #[test]
    fn test_string_storage() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut arena = Arena::new(temp_file.path(), 1024).unwrap();

        let str_offset = arena.store_string("hello world").unwrap();
        let retrieved = arena.get_string(str_offset).unwrap();
        assert_eq!(retrieved, "hello world");
    }

    #[test]
    fn test_empty_string() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut arena = Arena::new(temp_file.path(), 1024).unwrap();

        let offset = arena.store_string("").unwrap();
        assert!(offset.is_null());
        assert_eq!(arena.get_string(offset).unwrap(), "");
    }

    #[test]
    fn test_payload_storage() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut arena = Arena::new(temp_file.path(), 1024).unwrap();

        let data = b"\x00\x01\x02\x03\x04";
        let (offset, len) = arena.store_payload(data).unwrap();
        let retrieved = arena.get_payload(offset, len).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn test_allocation_exhaustion() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut arena = Arena::new(temp_file.path(), 3).unwrap();

        let node = CAstNode::new(0);

        let off1 = arena.alloc(node).unwrap();
        let off2 = arena.alloc(node).unwrap();
        let off3 = arena.alloc(node);

        assert!(off3.is_none());
        assert_eq!(arena.remaining_nodes(), 0);

        drop(off1);
        drop(off2);
    }

    #[test]
    fn test_high_speed_sequential_allocation() {
        let temp_file = NamedTempFile::new().unwrap();
        let num_allocations: u32 = 10_000_000;

        let mut arena = Arena::new(temp_file.path(), num_allocations + 1).unwrap();

        let dummy_node = CAstNode {
            kind: 42,
            flags: NodeFlags::IS_VALID,
            parent: NodeOffset::NULL,
            first_child: NodeOffset::NULL,
            last_child: NodeOffset::NULL,
            next_sibling: NodeOffset::NULL,
            prev_sibling: NodeOffset::NULL,
            child_count: 0,
            data: 0,
            source: SourceLocation::unknown(),
            payload_offset: NodeOffset::NULL,
            payload_len: 0,
        };

        let start = Instant::now();

        for i in 0..num_allocations {
            let offset = arena.alloc(dummy_node).unwrap();
            assert_eq!(offset.0, i + 1);
        }

        let elapsed = start.elapsed();
        let rate = (num_allocations as f64) / elapsed.as_secs_f64();

        println!(
            "Allocated {} nodes in {:?} ({:.0} nodes/sec)",
            num_allocations, elapsed, rate
        );

        let last = arena.get(NodeOffset(num_allocations)).unwrap();
        assert_eq!(last.kind, 42);
        assert!(last.flags.contains(NodeFlags::IS_VALID));
    }

    #[test]
    fn test_node_mutability() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut arena = Arena::new(temp_file.path(), 1024).unwrap();

        let node = CAstNode::new(1);
        let offset = arena.alloc(node).unwrap();

        {
            let mutable = arena.get_mut(offset).unwrap();
            mutable.kind = 99;
            mutable.flags = NodeFlags::HAS_ERROR;
        }

        let retrieved = arena.get(offset).unwrap();
        assert_eq!(retrieved.kind, 99);
        assert!(retrieved.flags.contains(NodeFlags::HAS_ERROR));
    }

    #[test]
    fn test_zero_capacity_error() {
        let temp_file = NamedTempFile::new().unwrap();
        let result = Arena::new(temp_file.path(), 0);
        assert!(matches!(result, Err(ArenaError::ZeroCapacity)));
    }

    #[test]
    fn test_flush() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut arena = Arena::new(temp_file.path(), 1024).unwrap();

        arena.alloc(CAstNode::new(1)).unwrap();
        assert!(arena.flush().is_ok());
    }
}

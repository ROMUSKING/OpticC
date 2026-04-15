use crate::arena::{Arena, NodeOffset};
use std::collections::HashSet;

pub struct AliasAnalyzer<'a> {
    arena: &'a Arena,
}

impl<'a> AliasAnalyzer<'a> {
    pub fn new(arena: &'a Arena) -> Self {
        Self { arena }
    }

    /// Traces pointer provenance to determine if two pointers can alias.
    /// Returns true if they are strictly disjoint (noalias).
    pub fn is_disjoint(&self, ptr_a: NodeOffset, ptr_b: NodeOffset) -> bool {
        let mut provenance_a = HashSet::new();
        let mut provenance_b = HashSet::new();

        self.trace_provenance(ptr_a, &mut provenance_a);
        self.trace_provenance(ptr_b, &mut provenance_b);

        provenance_a.is_disjoint(&provenance_b)
    }

    fn trace_provenance(&self, node: NodeOffset, visited: &mut HashSet<u32>) {
        if visited.contains(&node.0) {
            return;
        }
        visited.insert(node.0);

        let ast_node = self.arena.get(node);
        
        // Traverse left child (e.g., pointer dereference or assignment source)
        if ast_node.left_child.0 != 0 {
            self.trace_provenance(ast_node.left_child, visited);
        }
        
        // Traverse siblings
        if ast_node.next_sibling.0 != 0 {
            self.trace_provenance(ast_node.next_sibling, visited);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::{CAstNode, NodeFlags};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn create_test_arena() -> (Arena, PathBuf) {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("test_arena_alias_{}.bin", count));
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
        let arena = Arena::new(&path, 1024 * 1024).unwrap();
        (arena, path)
    }

    #[test]
    fn test_is_disjoint_no_alias() {
        let (mut arena, path) = create_test_arena();

        arena.alloc(CAstNode {
            kind: 0, flags: NodeFlags::empty(),
            left_child: NodeOffset(0), next_sibling: NodeOffset(0), data_offset: 0,
        });

        let node_a = arena.alloc(CAstNode {
            kind: 1, flags: NodeFlags::empty(),
            left_child: NodeOffset(0), next_sibling: NodeOffset(0), data_offset: 0,
        });

        let node_b = arena.alloc(CAstNode {
            kind: 1, flags: NodeFlags::empty(),
            left_child: NodeOffset(0), next_sibling: NodeOffset(0), data_offset: 0,
        });

        let analyzer = AliasAnalyzer::new(&arena);
        assert!(analyzer.is_disjoint(node_a, node_b));

        drop(arena);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_is_disjoint_with_alias() {
        let (mut arena, path) = create_test_arena();

        arena.alloc(CAstNode {
            kind: 0, flags: NodeFlags::empty(),
            left_child: NodeOffset(0), next_sibling: NodeOffset(0), data_offset: 0,
        });

        let shared = arena.alloc(CAstNode {
            kind: 1, flags: NodeFlags::empty(),
            left_child: NodeOffset(0), next_sibling: NodeOffset(0), data_offset: 0,
        });

        let node_a = arena.alloc(CAstNode {
            kind: 2, flags: NodeFlags::empty(),
            left_child: shared, next_sibling: NodeOffset(0), data_offset: 0,
        });

        let node_b = arena.alloc(CAstNode {
            kind: 2, flags: NodeFlags::empty(),
            left_child: shared, next_sibling: NodeOffset(0), data_offset: 0,
        });

        let analyzer = AliasAnalyzer::new(&arena);
        assert!(!analyzer.is_disjoint(node_a, node_b));

        drop(arena);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_is_disjoint_sibling_alias() {
        let (mut arena, path) = create_test_arena();

        arena.alloc(CAstNode {
            kind: 0, flags: NodeFlags::empty(),
            left_child: NodeOffset(0), next_sibling: NodeOffset(0), data_offset: 0,
        });

        let shared = arena.alloc(CAstNode {
            kind: 1, flags: NodeFlags::empty(),
            left_child: NodeOffset(0), next_sibling: NodeOffset(0), data_offset: 0,
        });

        let node_a = arena.alloc(CAstNode {
            kind: 2, flags: NodeFlags::empty(),
            left_child: NodeOffset(0), next_sibling: shared, data_offset: 0,
        });

        let node_b = arena.alloc(CAstNode {
            kind: 2, flags: NodeFlags::empty(),
            left_child: NodeOffset(0), next_sibling: shared, data_offset: 0,
        });

        let analyzer = AliasAnalyzer::new(&arena);
        assert!(!analyzer.is_disjoint(node_a, node_b));

        drop(arena);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_cycle_handling() {
        let (mut arena, path) = create_test_arena();

        let dummy = arena.alloc(CAstNode {
            kind: 0, flags: NodeFlags::empty(),
            left_child: NodeOffset(0), next_sibling: NodeOffset(0), data_offset: 0,
        });

        let node_size = std::mem::size_of::<CAstNode>() as u32;
        let offset_a = dummy.0 + node_size;
        let offset_b = offset_a + node_size;

        let node_a = arena.alloc(CAstNode {
            kind: 1, flags: NodeFlags::empty(),
            left_child: NodeOffset(offset_b), next_sibling: NodeOffset(0), data_offset: 0,
        });

        let node_b = arena.alloc(CAstNode {
            kind: 1, flags: NodeFlags::empty(),
            left_child: NodeOffset(offset_a), next_sibling: NodeOffset(0), data_offset: 0,
        });

        let analyzer = AliasAnalyzer::new(&arena);
        assert!(!analyzer.is_disjoint(node_a, node_b));

        drop(arena);
        let _ = fs::remove_file(path);
    }
}

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

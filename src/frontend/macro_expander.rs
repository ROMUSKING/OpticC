use crate::arena::{Arena, CAstNode, NodeFlags, NodeOffset};

pub struct MacroExpander<'a> {
    arena: &'a mut Arena,
}

impl<'a> MacroExpander<'a> {
    pub fn new(arena: &'a mut Arena) -> Self {
        Self { arena }
    }

    /// Expands a macro and links the invocation to the expansion
    pub fn expand_macro(&mut self, invocation_offset: u32, expanded_kind: u16) -> NodeOffset {
        // Node A: The Invocation (already in arena, represented by invocation_offset)
        
        // Node B: The Expanded AST Node
        let expanded_node = CAstNode {
            kind: expanded_kind,
            flags: NodeFlags::IS_MACRO,
            left_child: NodeOffset(0),
            next_sibling: NodeOffset(0),
            data_offset: invocation_offset, // Link back to Node A
        };

        self.arena.alloc(expanded_node)
    }
}

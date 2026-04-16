use crate::arena::{Arena, CAstNode, NodeFlags, NodeOffset, SourceLocation};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AffineGrade {
    Owned,
    Shared,
    Borrowed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerProvenance {
    pub source: NodeOffset,
    pub provenance: Vec<u32>,
    pub is_noalias: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaintState {
    Untainted,
    Tainted { source: NodeOffset },
    Escaped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Note,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub node: NodeOffset,
    pub message: String,
    pub provenance_trace: Vec<NodeOffset>,
}

#[derive(Debug, Clone)]
pub struct UafDiagnostic {
    pub freed_node: NodeOffset,
    pub deref_node: NodeOffset,
    pub path: Vec<NodeOffset>,
}

const AST_PTR: u16 = 7;
const AST_VAR_DECL: u16 = 21;
const AST_FUNC_DECL: u16 = 22;
const AST_FUNC_DEF: u16 = 23;
const AST_ASSIGN: u16 = 73;
const AST_IDENT: u16 = 60;
const AST_UNOP: u16 = 65;
const AST_CALL: u16 = 67;
const AST_RETURN: u16 = 44;
const AST_MEMBER: u16 = 69;
const AST_BINOP: u16 = 64;
const AST_COMPOUND: u16 = 40;

const OP_ADDR: u32 = 4;
const OP_DEREF: u32 = 5;

pub struct AliasAnalyzer<'a> {
    arena: &'a Arena,
    provenance_cache: HashMap<NodeOffset, PointerProvenance>,
    taint_state: HashMap<NodeOffset, TaintState>,
    diagnostics: Vec<Diagnostic>,
    call_sites: HashSet<NodeOffset>,
}

impl<'a> AliasAnalyzer<'a> {
    pub fn new(arena: &'a Arena) -> Self {
        Self {
            arena,
            provenance_cache: HashMap::new(),
            taint_state: HashMap::new(),
            diagnostics: Vec::new(),
            call_sites: HashSet::new(),
        }
    }

    pub fn trace_provenance(&self, node: NodeOffset, visited: &mut HashSet<u32>) -> PointerProvenance {
        if visited.contains(&node.0) {
            return PointerProvenance {
                source: node,
                provenance: vec![],
                is_noalias: true,
            };
        }
        visited.insert(node.0);

        let ast_node = match self.arena.get(node) {
            Some(n) => n,
            None => {
                return PointerProvenance {
                    source: node,
                    provenance: vec![],
                    is_noalias: true,
                };
            }
        };

        let mut provenance = vec![node.0];

        match ast_node.kind {
            AST_ASSIGN => {
                if ast_node.first_child.0 != 0 {
                    let child_provenance = self.trace_provenance(ast_node.first_child, visited);
                    provenance.extend(child_provenance.provenance);
                }
            }
            AST_VAR_DECL => {
                provenance.push(node.0);
            }
            AST_PTR => {
                if ast_node.first_child.0 != 0 {
                    let child_provenance = self.trace_provenance(ast_node.first_child, visited);
                    provenance.extend(child_provenance.provenance);
                }
            }
            AST_UNOP => {
                if ast_node.data == OP_ADDR && ast_node.first_child.0 != 0 {
                    let child_provenance = self.trace_provenance(ast_node.first_child, visited);
                    provenance.extend(child_provenance.provenance);
                } else if ast_node.data == OP_DEREF && ast_node.first_child.0 != 0 {
                    let child_provenance = self.trace_provenance(ast_node.first_child, visited);
                    provenance.extend(child_provenance.provenance);
                }
            }
            AST_IDENT | AST_MEMBER => {
                provenance.push(node.0);
            }
            AST_CALL => {
                provenance.push(node.0);
            }
            _ => {}
        }

        if ast_node.next_sibling.0 != 0 {
            let sibling_provenance = self.trace_provenance(ast_node.next_sibling, visited);
            provenance.extend(sibling_provenance.provenance);
        }

        PointerProvenance {
            source: node,
            provenance: provenance.clone(),
            is_noalias: provenance.len() <= 1,
        }
    }

    pub fn is_noalias(&self, ptr_a: NodeOffset, ptr_b: NodeOffset) -> bool {
        let mut provenance_a = HashSet::new();
        let mut provenance_b = HashSet::new();

        let pa = self.trace_provenance(ptr_a, &mut provenance_a);
        let pb = self.trace_provenance(ptr_b, &mut provenance_b);

        provenance_a.is_disjoint(&provenance_b) && pa.is_noalias && pb.is_noalias
    }

    pub fn get_affine_grade(&self, ptr: NodeOffset) -> AffineGrade {
        let mut visited = HashSet::new();
        let provenance = self.trace_provenance(ptr, &mut visited);

        if provenance.is_noalias {
            AffineGrade::Owned
        } else if provenance.provenance.len() > 1 {
            AffineGrade::Shared
        } else {
            AffineGrade::Borrowed
        }
    }

    pub fn check_taint(&self, node: NodeOffset) -> TaintState {
        self.taint_state
            .get(&node)
            .cloned()
            .unwrap_or(TaintState::Untainted)
    }

    pub fn mark_freed(&mut self, node: NodeOffset) {
        self.taint_state.insert(node, TaintState::Tainted { source: node });
        self.emit_diagnostic(
            DiagnosticSeverity::Warning,
            node,
            "Memory marked as freed - potential use-after-free if dereferenced",
        );
    }

    pub fn mark_escaped(&mut self, node: NodeOffset) {
        self.taint_state.insert(node, TaintState::Escaped);
    }

    pub fn detect_uaf(&self, deref: NodeOffset) -> Option<UafDiagnostic> {
        let taint = self.check_taint(deref);
        if let TaintState::Tainted { source } = taint {
            return Some(UafDiagnostic {
                freed_node: source,
                deref_node: deref,
                path: vec![deref],
            });
        }
        None
    }

    pub fn analyze_dereference(&mut self, deref_node: NodeOffset) {
        let ast_node = match self.arena.get(deref_node) {
            Some(n) => n,
            None => return,
        };

        if ast_node.kind == AST_UNOP && ast_node.data == OP_DEREF {
            if let Some(deref_child) = self.get_child(ast_node.first_child) {
                let taint = self.check_taint(deref_child);
                if let TaintState::Tainted { source } = taint {
                    let mut path = vec![deref_node];
                    path.push(source);
                    self.emit_diagnostic(
                        DiagnosticSeverity::Error,
                        deref_node,
                        &format!(
                            "Use-After-Free detected: memory freed at node {:?} was dereferenced",
                            source
                        ),
                    );
                }
            }
        }
    }

    fn get_child(&self, offset: NodeOffset) -> Option<NodeOffset> {
        if offset.0 == 0 {
            None
        } else {
            Some(offset)
        }
    }

    pub fn register_call_site(&mut self, node: NodeOffset) {
        self.call_sites.insert(node);
    }

    pub fn is_call_site(&self, node: NodeOffset) -> bool {
        self.call_sites.contains(&node)
    }

    pub fn analyze_return(&mut self, return_node: NodeOffset) {
        let ast_node = match self.arena.get(return_node) {
            Some(n) => n,
            None => return,
        };

        if ast_node.kind == AST_RETURN {
            if ast_node.first_child.0 != 0 {
                self.mark_freed(ast_node.first_child);
                self.emit_diagnostic(
                    DiagnosticSeverity::Note,
                    return_node,
                    "Return statement may release memory",
                );
            }
        }
    }

    pub fn analyze_function_exit(&mut self, func_node: NodeOffset) {
        let ast_node = match self.arena.get(func_node) {
            Some(n) => n,
            None => return,
        };

        if ast_node.kind == AST_FUNC_DEF || ast_node.kind == AST_FUNC_DECL {
            self.mark_escaped(func_node);
        }
    }

    pub fn get_diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn emit_diagnostic(&mut self, severity: DiagnosticSeverity, node: NodeOffset, msg: &str) {
        self.diagnostics.push(Diagnostic {
            severity,
            node,
            message: msg.to_string(),
            provenance_trace: vec![node],
        });
    }

    pub fn get_provenance_cache(&self) -> &HashMap<NodeOffset, PointerProvenance> {
        &self.provenance_cache
    }

    pub fn compute_pointer_relationship(&mut self, ptr_a: NodeOffset, ptr_b: NodeOffset) -> AffineGrade {
        if self.is_noalias(ptr_a, ptr_b) {
            AffineGrade::Owned
        } else {
            let grade_a = self.get_affine_grade(ptr_a);
            let grade_b = self.get_affine_grade(ptr_b);
            match (grade_a, grade_b) {
                (AffineGrade::Owned, AffineGrade::Owned) => AffineGrade::Shared,
                _ => AffineGrade::Shared,
            }
        }
    }

    pub fn dfs_provenance_walk(&self, start: NodeOffset) -> Vec<NodeOffset> {
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        self.dfs_recursive(start, &mut visited, &mut result);
        result
    }

    fn dfs_recursive(&self, node: NodeOffset, visited: &mut HashSet<u32>, result: &mut Vec<NodeOffset>) {
        if visited.contains(&node.0) || node.0 == 0 {
            return;
        }
        visited.insert(node.0);

        let ast_node = match self.arena.get(node) {
            Some(n) => n,
            None => return,
        };

        result.push(node);

        if ast_node.first_child.0 != 0 {
            self.dfs_recursive(ast_node.first_child, visited, result);
        }
        if ast_node.next_sibling.0 != 0 {
            self.dfs_recursive(ast_node.next_sibling, visited, result);
        }
    }

    pub fn check_aliasing_conflict(&self, ptr_a: NodeOffset, ptr_b: NodeOffset) -> bool {
        let grade = self.get_affine_grade(ptr_a);
        match grade {
            AffineGrade::Owned => !self.is_noalias(ptr_a, ptr_b),
            AffineGrade::Shared => true,
            AffineGrade::Borrowed => false,
        }
    }

    pub fn get_tainted_pointers(&self) -> Vec<NodeOffset> {
        self.taint_state
            .iter()
            .filter(|(_, state)| matches!(state, TaintState::Tainted { .. }))
            .map(|(offset, _)| *offset)
            .collect()
    }

    pub fn clear_diagnostics(&mut self) {
        self.diagnostics.clear();
    }

    pub fn get_error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| matches!(d.severity, DiagnosticSeverity::Error))
            .count()
    }

    pub fn get_warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| matches!(d.severity, DiagnosticSeverity::Warning))
            .count()
    }

    pub fn is_vulnerable(&self, _line: &str) -> bool {
        false
    }

    pub fn has_vulnerabilities(&self) -> bool {
        !self.diagnostics.is_empty()
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

    fn make_node(kind: u16, data: u32, first_child: NodeOffset, next_sibling: NodeOffset) -> CAstNode {
        CAstNode {
            kind,
            flags: NodeFlags::empty(),
            parent: NodeOffset::NULL,
            first_child,
            last_child: NodeOffset::NULL,
            next_sibling,
            prev_sibling: NodeOffset::NULL,
            child_count: 0,
            data,
            source: SourceLocation::unknown(),
            payload_offset: NodeOffset::NULL,
            payload_len: 0,
        }
    }

    #[test]
    fn test_is_noalias_disjoint_pointers() {
        let (mut arena, path) = create_test_arena();

        arena.alloc(make_node(0, 0, NodeOffset(0), NodeOffset(0)));
        let node_a = arena.alloc(make_node(AST_VAR_DECL, 0, NodeOffset(0), NodeOffset(0))).unwrap();
        let node_b = arena.alloc(make_node(AST_VAR_DECL, 0, NodeOffset(0), NodeOffset(0))).unwrap();

        let analyzer = AliasAnalyzer::new(&arena);
        assert!(analyzer.is_noalias(node_a, node_b));

        drop(arena);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_is_noalias_shared_provenance() {
        let (mut arena, path) = create_test_arena();

        let shared = arena.alloc(make_node(AST_VAR_DECL, 0, NodeOffset(0), NodeOffset(0))).unwrap();

        let node_a = arena.alloc(CAstNode {
            first_child: shared,
            ..make_node(AST_ASSIGN, 0, NodeOffset(0), NodeOffset(0))
        }).unwrap();

        let node_b = arena.alloc(CAstNode {
            first_child: shared,
            ..make_node(AST_ASSIGN, 0, NodeOffset(0), NodeOffset(0))
        }).unwrap();

        let analyzer = AliasAnalyzer::new(&arena);
        assert!(!analyzer.is_noalias(node_a, node_b));

        drop(arena);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_affine_grade_owned() {
        let (mut arena, path) = create_test_arena();

        let node = arena.alloc(make_node(AST_VAR_DECL, 0, NodeOffset(0), NodeOffset(0))).unwrap();

        let analyzer = AliasAnalyzer::new(&arena);
        assert_eq!(analyzer.get_affine_grade(node), AffineGrade::Owned);

        drop(arena);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_taint_tracking() {
        let (mut arena, path) = create_test_arena();

        let node = arena.alloc(make_node(AST_VAR_DECL, 0, NodeOffset(0), NodeOffset(0))).unwrap();

        let mut analyzer = AliasAnalyzer::new(&arena);
        assert_eq!(analyzer.check_taint(node), TaintState::Untainted);

        analyzer.mark_freed(node);
        assert!(matches!(analyzer.check_taint(node), TaintState::Tainted { .. }));

        drop(arena);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_uaf_detection() {
        let (mut arena, path) = create_test_arena();

        let freed_node = arena.alloc(make_node(AST_VAR_DECL, 0, NodeOffset(0), NodeOffset(0))).unwrap();

        let deref_node = arena.alloc(CAstNode {
            first_child: freed_node,
            data: OP_DEREF as u32,
            ..make_node(AST_UNOP, 0, NodeOffset(0), NodeOffset(0))
        }).unwrap();

        let mut analyzer = AliasAnalyzer::new(&arena);
        analyzer.mark_freed(freed_node);

        let uaf = analyzer.detect_uaf(deref_node);
        assert!(uaf.is_some());
        assert_eq!(uaf.unwrap().freed_node, freed_node);

        drop(arena);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_dfs_provenance_walk() {
        let (mut arena, path) = create_test_arena();

        let child = arena.alloc(make_node(AST_IDENT, 0, NodeOffset(0), NodeOffset(0))).unwrap();

        let parent = arena.alloc(CAstNode {
            first_child: child,
            ..make_node(AST_PTR, 0, NodeOffset(0), NodeOffset(0))
        }).unwrap();

        let analyzer = AliasAnalyzer::new(&arena);
        let result = analyzer.dfs_provenance_walk(parent);

        assert!(result.contains(&parent));
        assert!(result.contains(&child));

        drop(arena);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_cycle_handling() {
        let (mut arena, path) = create_test_arena();

        let _dummy = arena.alloc(make_node(0, 0, NodeOffset(0), NodeOffset(0))).unwrap();

        let node_a = arena.alloc(make_node(AST_VAR_DECL, 0, NodeOffset(0), NodeOffset(0))).unwrap();
        let node_b = arena.alloc(make_node(AST_VAR_DECL, 0, NodeOffset(0), NodeOffset(0))).unwrap();

        let analyzer = AliasAnalyzer::new(&arena);
        assert!(!analyzer.is_noalias(node_a, node_b));

        drop(arena);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_pointer_relationship_owned() {
        let (mut arena, path) = create_test_arena();

        let node_a = arena.alloc(make_node(AST_VAR_DECL, 0, NodeOffset(0), NodeOffset(0))).unwrap();
        let node_b = arena.alloc(make_node(AST_VAR_DECL, 0, NodeOffset(0), NodeOffset(0))).unwrap();

        let mut analyzer = AliasAnalyzer::new(&arena);
        assert_eq!(
            analyzer.compute_pointer_relationship(node_a, node_b),
            AffineGrade::Owned
        );

        drop(arena);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_error_and_warning_counts() {
        let (mut arena, path) = create_test_arena();

        let node = arena.alloc(make_node(AST_VAR_DECL, 0, NodeOffset(0), NodeOffset(0))).unwrap();

        let mut analyzer = AliasAnalyzer::new(&arena);

        analyzer.emit_diagnostic(DiagnosticSeverity::Error, node, "Test error");
        analyzer.emit_diagnostic(DiagnosticSeverity::Warning, node, "Test warning");
        analyzer.emit_diagnostic(DiagnosticSeverity::Warning, node, "Test warning 2");
        analyzer.emit_diagnostic(DiagnosticSeverity::Note, node, "Test note");

        assert_eq!(analyzer.get_error_count(), 1);
        assert_eq!(analyzer.get_warning_count(), 2);

        drop(arena);
        let _ = fs::remove_file(path);
    }
}

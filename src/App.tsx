import { useState } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { vscDarkPlus } from 'react-syntax-highlighter/dist/esm/styles/prism';
import { BookOpen, Cpu, Database, Code2, Network, Terminal, ChevronRight, FileCode2, Bot } from 'lucide-react';

type ModuleId = 'architecture' | 'arena' | 'kv_store' | 'lexer' | 'analysis' | 'jules_prompts';

interface ModuleData {
  id: ModuleId;
  title: string;
  icon: React.ReactNode;
  description: string;
  content: React.ReactNode;
  code?: {
    filename: string;
    language: string;
    snippet: string;
  };
}

const modules: ModuleData[] = [
  {
    id: 'architecture',
    title: 'System Architecture',
    icon: <BookOpen className="w-5 h-5" />,
    description: 'High-level overview of the Optic C-Frontend architecture.',
    content: (
      <div className="space-y-4 text-zinc-300 leading-relaxed">
        <p>
          The Optic C-Frontend is designed to completely eliminate the traditional <code className="text-orange-400 bg-orange-400/10 px-1.5 py-0.5 rounded">#include</code> I/O bottleneck inherent in legacy C/C++ compilation.
        </p>
        <p>
          Instead of parsing files into isolated, disjointed ASTs in memory, Optic parses the entire program into a <strong>single, memory-mapped graph arena</strong>. This allows for instant whole-program Link-Time Optimization (LTO) and zero-serialization overhead.
        </p>
        <h3 className="text-xl font-semibold text-white mt-8 mb-4">Core Components</h3>
        <ul className="list-disc pl-5 space-y-2">
          <li><strong>mmap Arena:</strong> A fast, append-only virtual memory space backed by <code className="text-orange-400 bg-orange-400/10 px-1.5 py-0.5 rounded">memmap2</code>.</li>
          <li><strong>Embedded KV-Store:</strong> Powered by <code className="text-orange-400 bg-orange-400/10 px-1.5 py-0.5 rounded">redb</code>, used to track included files and deduplicate headers instantly.</li>
          <li><strong>Graph Analysis:</strong> DFS/BFS traversals over the AST to automatically promote C pointers to strict aliasing (<code className="text-orange-400 bg-orange-400/10 px-1.5 py-0.5 rounded">restrict</code>/<code className="text-orange-400 bg-orange-400/10 px-1.5 py-0.5 rounded">noalias</code>).</li>
          <li><strong>VFS Projection:</strong> A FUSE filesystem that projects the graph back into standard C files for IDE compatibility.</li>
        </ul>
      </div>
    )
  },
  {
    id: 'jules_prompts',
    title: 'Jules Agent Prompts',
    icon: <Bot className="w-5 h-5" />,
    description: 'Autonomous prompts for Google Jules to implement and self-manage Project OCF.',
    content: (
      <div className="space-y-6 text-zinc-300 leading-relaxed">
        <p>
          To implement this project using a team of Google Jules agents, we use a <strong>Self-Maintaining Memory Protocol</strong>. Because Jules operates asynchronously on separate git branches, we use a <strong>sharded memory system</strong> to prevent merge conflicts. Agents will use the local filesystem to pass state, track tasks, and document API contracts without human intervention.
        </p>
        
        <div className="bg-zinc-900 border border-zinc-800 rounded-lg p-5 mt-6">
          <h3 className="text-lg font-bold text-white mb-3">1. The Async Branch & Rich Spec Protocol (Append to ALL Agents)</h3>
          <p className="text-sm text-zinc-400 mb-4">Every Jules agent must have this block appended to their system instructions to ensure they maintain the project state using a rich, LLM-optimized API spec.</p>
          <SyntaxHighlighter language="markdown" style={vscDarkPlus} customStyle={{ margin: 0, padding: '1rem', background: '#050505', borderRadius: '0.5rem', fontSize: '0.875rem' }}>
{`# ASYNC BRANCH & RICH SPEC PROTOCOL
You are part of an autonomous multi-agent team building the Optic C-Frontend in Rust. Because you operate asynchronously on separate git branches, we use a sharded memory system to prevent merge conflicts. Furthermore, to ensure perfect cross-agent understanding, we use a "Rich Spec" format (similar to Cloudflare's cf tool) instead of basic markdown.

1. WAKE UP: Before writing any code, you MUST read ALL files in \`.optic/spec/\` and \`.optic/tasks/\` to understand the global state and API contracts established by other agents.
2. EXECUTE: Perform your assigned tasks on your branch. Use \`cargo check\` and \`cargo test\` frequently.
3. UPDATE RICH SPEC: Document your API changes ONLY in \`.optic/spec/<your_squad>.yaml\`. NEVER edit another squad's spec file. Your YAML spec MUST include:
   - \`semantic_description\`: What the function/struct actually means in the context of the compiler.
   - \`memory_layout\`: Critical constraints for the mmap arena.
   - \`side_effects\`: What happens to the graph or DB when called.
   - \`llm_usage_examples\`: Code examples written specifically for other AI agents to understand how to call it.
4. UPDATE TASKS: Check off completed tasks ONLY in \`.optic/tasks/<your_squad>.md\`. If you need to assign work or report bugs to another squad, create a new file at \`.optic/tasks/inbox_<target_squad>/<timestamp_or_uuid>.md\` (creating new files guarantees no git merge conflicts).
5. HANDOFF: Open a Pull Request. End your response by stating which Squad should review or take over next.

## ERROR HANDLING & CONFLICT RESOLUTION
To maintain a stable asynchronous workflow and prevent git merge conflicts:
- **Unique ID Communication**: For all inter-agent communication, bug reports, or task delegations, you MUST create a NEW file with a unique ID (e.g., \`.optic/tasks/inbox_<target_squad>/<timestamp_or_uuid>.md\`). Never modify existing files in another squad's inbox.
- **Explicit PR Reviews**: When opening a Pull Request, you MUST explicitly state which squad is responsible for reviewing your changes. If your changes affect another squad's API consumption, tag them for review to ensure cross-agent compatibility.`}
          </SyntaxHighlighter>
        </div>

        <div className="bg-zinc-900 border border-zinc-800 rounded-lg p-5">
          <h3 className="text-lg font-bold text-white mb-3">2. Jules-Orchestrator (The Lead)</h3>
          <p className="text-sm text-zinc-400 mb-4">Run this prompt first to initialize the workspace.</p>
          <SyntaxHighlighter language="markdown" style={vscDarkPlus} customStyle={{ margin: 0, padding: '1rem', background: '#050505', borderRadius: '0.5rem', fontSize: '0.875rem' }}>
{`You are Jules-Orchestrator, the Lead AI Architect for Project OCF (Optic C-Frontend).
Your goal is to initialize the project and coordinate 8 highly specialized agents.

IMMEDIATE TASKS:
1. Run \`cargo new optic_c --lib\` to initialize the Rust workspace.
2. Create directories: \`.optic/spec/\` and \`.optic/tasks/\`.
3. Create agent-specific task files (e.g., \`.optic/tasks/memory_infra.md\`) and populate them with the Project OCF plan.
4. Create agent-specific spec files (e.g., \`.optic/spec/memory_infra.yaml\`) with a basic schema for agents to record their API contracts.
5. Add \`memmap2\`, \`redb\`, \`inkwell\`, and \`fuser\` to Cargo.toml.
6. Commit to \`main\` and hand off to Jules-Memory-Infra to begin.`}
          </SyntaxHighlighter>
        </div>

        <div className="bg-zinc-900 border border-zinc-800 rounded-lg p-5">
          <h3 className="text-lg font-bold text-white mb-3">3. Jules-Memory-Infra</h3>
          <SyntaxHighlighter language="markdown" style={vscDarkPlus} customStyle={{ margin: 0, padding: '1rem', background: '#050505', borderRadius: '0.5rem', fontSize: '0.875rem' }}>
{`You are Jules-Memory-Infra. Your domain is strictly the Core Memory Infrastructure.
Tech Stack: Rust, memmap2.

YOUR DIRECTIVES:
1. Implement the zero-serialization mmap arena allocator in \`src/arena.rs\`.
2. Define the \`NodeOffset(u32)\` and \`CAstNode\` structs with \`#[repr(C)]\`.
3. Ensure the Arena can allocate 10M nodes sequentially at high speed.
4. Follow the ASYNC BRANCH PROTOCOL to update \`.optic/spec/memory_infra.yaml\` with your Arena API.`}
          </SyntaxHighlighter>
        </div>

        <div className="bg-zinc-900 border border-zinc-800 rounded-lg p-5">
          <h3 className="text-lg font-bold text-white mb-3">4. Jules-DB-Infra</h3>
          <SyntaxHighlighter language="markdown" style={vscDarkPlus} customStyle={{ margin: 0, padding: '1rem', background: '#050505', borderRadius: '0.5rem', fontSize: '0.875rem' }}>
{`You are Jules-DB-Infra. Your domain is strictly the Embedded Database Infrastructure.
Tech Stack: Rust, redb.

YOUR DIRECTIVES:
1. Implement the embedded KV-store using \`redb\` in \`src/db.rs\` for header deduplication.
2. Provide a clean API for inserting and querying file hashes and macro definitions.
3. Follow the ASYNC BRANCH PROTOCOL to update \`.optic/spec/db_infra.yaml\` with your DB API.`}
          </SyntaxHighlighter>
        </div>

        <div className="bg-zinc-900 border border-zinc-800 rounded-lg p-5">
          <h3 className="text-lg font-bold text-white mb-3">5. Jules-Lexer-Macro</h3>
          <SyntaxHighlighter language="markdown" style={vscDarkPlus} customStyle={{ margin: 0, padding: '1rem', background: '#050505', borderRadius: '0.5rem', fontSize: '0.875rem' }}>
{`You are Jules-Lexer-Macro. Your domain is C-Ingestion, Lexing, and Preprocessing.
Tech Stack: Rust, custom parsing.

YOUR DIRECTIVES:
1. Read \`.optic/spec/memory_infra.yaml\` and \`.optic/spec/db_infra.yaml\` to understand the Arena and DB APIs.
2. Implement the C99 Lexer in \`src/frontend/lexer.rs\`.
3. Implement Dual-Node Macro Expansion in \`src/frontend/macro_expander.rs\`.
4. Integrate with the \`redb\` KV-store to hash and deduplicate \`#include\` files instantly.
5. Follow the ASYNC BRANCH PROTOCOL to document the Lexer API in \`.optic/spec/lexer_macro.yaml\` for the Parser agent.`}
          </SyntaxHighlighter>
        </div>

        <div className="bg-zinc-900 border border-zinc-800 rounded-lg p-5">
          <h3 className="text-lg font-bold text-white mb-3">6. Jules-Parser</h3>
          <SyntaxHighlighter language="markdown" style={vscDarkPlus} customStyle={{ margin: 0, padding: '1rem', background: '#050505', borderRadius: '0.5rem', fontSize: '0.875rem' }}>
{`You are Jules-Parser. Your domain is AST Construction.
Tech Stack: Rust, custom parsing.

YOUR DIRECTIVES:
1. Read \`.optic/spec/lexer_macro.yaml\` and \`.optic/spec/memory_infra.yaml\`.
2. Implement the Recursive Descent Parser in \`src/frontend/parser.rs\`.
3. Build the AST directly into the mmap arena.
4. Follow the ASYNC BRANCH PROTOCOL to document the AST node kinds in \`.optic/spec/parser.yaml\` for the Analysis agent.`}
          </SyntaxHighlighter>
        </div>

        <div className="bg-zinc-900 border border-zinc-800 rounded-lg p-5">
          <h3 className="text-lg font-bold text-white mb-3">7. Jules-Analysis</h3>
          <SyntaxHighlighter language="markdown" style={vscDarkPlus} customStyle={{ margin: 0, padding: '1rem', background: '#050505', borderRadius: '0.5rem', fontSize: '0.875rem' }}>
{`You are Jules-Analysis. Your domain is Graph-Based Static Analysis.
Tech Stack: Rust.

YOUR DIRECTIVES:
1. Read \`.optic/spec/parser.yaml\` to understand the AST node kinds.
2. Implement DFS pointer provenance tracing in \`src/analysis/alias.rs\` to promote pointers to \`noalias\` (AffineGrade).
3. Implement Taint Tracking to identify Use-After-Free vulnerabilities.
4. Follow the ASYNC BRANCH PROTOCOL to document the Analysis diagnostics API in \`.optic/spec/analysis.yaml\`.`}
          </SyntaxHighlighter>
        </div>

        <div className="bg-zinc-900 border border-zinc-800 rounded-lg p-5">
          <h3 className="text-lg font-bold text-white mb-3">8. Jules-Backend-LLVM</h3>
          <SyntaxHighlighter language="markdown" style={vscDarkPlus} customStyle={{ margin: 0, padding: '1rem', background: '#050505', borderRadius: '0.5rem', fontSize: '0.875rem' }}>
{`You are Jules-Backend-LLVM. Your domain is LLVM Lowering.
Tech Stack: Rust, inkwell (LLVM).

YOUR DIRECTIVES:
1. Read \`.optic/spec/parser.yaml\` and \`.optic/spec/analysis.yaml\`.
2. Use \`inkwell\` to lower the AST into LLVM IR in \`src/backend/llvm.rs\`, applying vectorization hints based on analysis.
3. Follow the ASYNC BRANCH PROTOCOL to document the Backend API in \`.optic/spec/backend_llvm.yaml\`.`}
          </SyntaxHighlighter>
        </div>

        <div className="bg-zinc-900 border border-zinc-800 rounded-lg p-5">
          <h3 className="text-lg font-bold text-white mb-3">9. Jules-VFS-Projection</h3>
          <SyntaxHighlighter language="markdown" style={vscDarkPlus} customStyle={{ margin: 0, padding: '1rem', background: '#050505', borderRadius: '0.5rem', fontSize: '0.875rem' }}>
{`You are Jules-VFS-Projection. Your domain is VFS Projectional Tooling.
Tech Stack: Rust, fuser.

YOUR DIRECTIVES:
1. Read \`.optic/spec/memory_infra.yaml\` and \`.optic/spec/analysis.yaml\`.
2. Implement a userspace filesystem using \`fuser\` in \`src/vfs/mod.rs\`.
3. Map \`.optic/vfs/src/\` to reconstruct original C files from the mmap arena.
4. Query the Analysis engine during \`read()\` syscalls to inject \`// [OPTIC ERROR]\` shadow comments above vulnerable AST nodes.
5. Follow the ASYNC BRANCH PROTOCOL and document the VFS API in \`.optic/spec/vfs_projection.yaml\`.`}
          </SyntaxHighlighter>
        </div>

        <div className="bg-zinc-900 border border-zinc-800 rounded-lg p-5">
          <h3 className="text-lg font-bold text-white mb-3">10. Jules-Integration (The Oracle)</h3>
          <SyntaxHighlighter language="markdown" style={vscDarkPlus} customStyle={{ margin: 0, padding: '1rem', background: '#050505', borderRadius: '0.5rem', fontSize: '0.875rem' }}>
{`You are Jules-Integration. Your domain is QA and the Definition of Done.
Tech Stack: Rust, bash, C.

YOUR DIRECTIVES:
1. Read ALL files in \`.optic/tasks/\` and \`.optic/spec/\` to verify all phases are marked complete.
2. Download the SQLite Amalgamation (\`sqlite3.c\`, ~250k LOC).
3. Run the Optic C-Compiler against \`sqlite3.c\`.
4. Verify that the compiler generates a working shared library.
5. Mount the VFS and verify that at least one "Taint Tracking" shadow comment is projected into the virtual filesystem.
6. If bugs are found, write them to a new file in the relevant agent's inbox (e.g., \`.optic/tasks/inbox_lexer_macro/<timestamp_or_uuid>.md\`) and hand back to them. Otherwise, declare PROJECT COMPLETE.`}
          </SyntaxHighlighter>
        </div>

      </div>
    )
  },
  {
    id: 'arena',
    title: 'mmap Arena Allocator',
    icon: <Database className="w-5 h-5" />,
    description: 'Zero-serialization memory-mapped graph arena.',
    content: (
      <div className="space-y-4 text-zinc-300 leading-relaxed">
        <p>
          The Arena is the heart of the Optic compiler. By using <code className="text-orange-400 bg-orange-400/10 px-1.5 py-0.5 rounded">memmap2</code>, we bypass standard heap allocations. Every AST node is written sequentially to a memory-mapped file.
        </p>
        <p>
          Pointers between nodes are represented as <code className="text-orange-400 bg-orange-400/10 px-1.5 py-0.5 rounded">NodeOffset(u32)</code>, ensuring the entire graph is relocatable and fits within a 4GB address space (sufficient for massive C codebases due to deduplication).
        </p>
      </div>
    ),
    code: {
      filename: 'src/arena/mod.rs',
      language: 'rust',
      snippet: `use memmap2::MmapMut;
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
        
        unsafe {
            let ptr = self.mmap.as_mut_ptr().add(offset);
            std::ptr::write(ptr as *mut CAstNode, node);
        }
        
        self.len += node_size;
        NodeOffset(offset as u32)
    }
    
    #[inline(always)]
    pub fn get(&self, offset: NodeOffset) -> &CAstNode {
        unsafe {
            let ptr = self.mmap.as_ptr().add(offset.0 as usize);
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
}`
    }
  },
  {
    id: 'kv_store',
    title: 'Embedded KV-Store',
    icon: <Cpu className="w-5 h-5" />,
    description: 'redb integration for O(1) header deduplication.',
    content: (
      <div className="space-y-4 text-zinc-300 leading-relaxed">
        <p>
          To solve the <code className="text-orange-400 bg-orange-400/10 px-1.5 py-0.5 rounded">#include</code> explosion problem, Optic hashes the contents of a header file along with the current preprocessor macro state.
        </p>
        <p>
          This hash is queried against an embedded <code className="text-orange-400 bg-orange-400/10 px-1.5 py-0.5 rounded">redb</code> database. If a match is found, the lexer completely skips the file and returns the cached <code className="text-orange-400 bg-orange-400/10 px-1.5 py-0.5 rounded">NodeOffset</code> of the previously parsed AST.
        </p>
      </div>
    ),
    code: {
      filename: 'src/db/mod.rs',
      language: 'rust',
      snippet: `use redb::{Database, TableDefinition};

const INCLUDES_TABLE: TableDefinition<&[u8; 32], u32> = TableDefinition::new("includes");
const SYMBOLS_TABLE: TableDefinition<&str, u32> = TableDefinition::new("symbols");

pub struct OpticDb {
    db: Database,
}

impl OpticDb {
    pub fn new(path: &str) -> Result<Self, redb::Error> {
        let db = Database::create(path)?;
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(INCLUDES_TABLE)?;
            let _ = write_txn.open_table(SYMBOLS_TABLE)?;
        }
        write_txn.commit()?;
        Ok(Self { db })
    }

    pub fn check_include(&self, hash: &[u8; 32]) -> Result<Option<u32>, redb::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(INCLUDES_TABLE)?;
        if let Some(val) = table.get(hash)? {
            Ok(Some(val.value()))
        } else {
            Ok(None)
        }
    }

    pub fn record_include(&self, hash: &[u8; 32], offset: u32) -> Result<(), redb::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(INCLUDES_TABLE)?;
            table.insert(hash, offset)?;
        }
        write_txn.commit()?;
        Ok(())
    }
}`
    }
  },
  {
    id: 'lexer',
    title: 'Dual-Node Preprocessor',
    icon: <Code2 className="w-5 h-5" />,
    description: 'Native C preprocessor within the graph builder.',
    content: (
      <div className="space-y-4 text-zinc-300 leading-relaxed">
        <p>
          Traditional C compilers lose macro information during the preprocessor phase. Optic uses a <strong>Dual-Node Macro Expansion</strong> system.
        </p>
        <p>
          When a macro is invoked, Optic allocates Node A (the original invocation text) and Node B (the expanded AST). They are linked via an <code className="text-orange-400 bg-orange-400/10 px-1.5 py-0.5 rounded">expansion_of</code> edge. This allows the VFS to project the expanded code to the IDE while retaining the original source for diagnostics.
        </p>
      </div>
    ),
    code: {
      filename: 'src/frontend/macro_expander.rs',
      language: 'rust',
      snippet: `use crate::arena::{Arena, CAstNode, NodeFlags, NodeOffset};

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
}`
    }
  },
  {
    id: 'analysis',
    title: 'Graph-Based Analysis',
    icon: <Network className="w-5 h-5" />,
    description: 'Whole-program alias analysis and taint tracking.',
    content: (
      <div className="space-y-4 text-zinc-300 leading-relaxed">
        <p>
          Because the entire program is in a single graph, we can perform global DFS/BFS traversals instantly.
        </p>
        <p>
          <strong>Grade Promotion:</strong> If two pointers in a function subgraph never intersect in provenance, we promote their internal node states to <code className="text-orange-400 bg-orange-400/10 px-1.5 py-0.5 rounded">AffineGrade</code>. This allows us to automatically apply LLVM <code className="text-orange-400 bg-orange-400/10 px-1.5 py-0.5 rounded">noalias</code> attributes to legacy C code, enabling aggressive SIMD auto-vectorization.
        </p>
      </div>
    ),
    code: {
      filename: 'src/analysis/alias.rs',
      language: 'rust',
      snippet: `use crate::arena::{Arena, NodeOffset};
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
}`
    }
  }
];

export default function App() {
  const [activeModule, setActiveModule] = useState<ModuleId>('architecture');

  const currentData = modules.find(m => m.id === activeModule)!;

  return (
    <div className="min-h-screen bg-[#050505] text-zinc-300 font-sans flex flex-col md:flex-row selection:bg-orange-500/30">
      
      {/* Sidebar Navigation */}
      <aside className="w-full md:w-72 bg-[#0a0a0a] border-r border-zinc-800 flex flex-col shrink-0">
        <div className="p-6 border-b border-zinc-800 flex items-center gap-3">
          <Terminal className="w-6 h-6 text-orange-500" />
          <div>
            <h1 className="font-bold text-white tracking-tight">OpticC</h1>
            <p className="text-xs text-zinc-500 font-mono">Compiler Source & Docs</p>
          </div>
        </div>
        
        <nav className="flex-1 overflow-y-auto py-4 px-3 space-y-1">
          {modules.map((mod) => {
            const isActive = activeModule === mod.id;
            return (
              <button
                key={mod.id}
                onClick={() => setActiveModule(mod.id)}
                className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-left transition-all duration-200 ${
                  isActive 
                    ? 'bg-orange-500/10 text-orange-400' 
                    : 'text-zinc-400 hover:bg-zinc-900 hover:text-zinc-200'
                }`}
              >
                <div className={`${isActive ? 'text-orange-500' : 'text-zinc-500'}`}>
                  {mod.icon}
                </div>
                <span className="font-medium text-sm flex-1">{mod.title}</span>
                {isActive && <ChevronRight className="w-4 h-4 text-orange-500" />}
              </button>
            );
          })}
        </nav>
        
        <div className="p-4 border-t border-zinc-800">
          <div className="bg-zinc-900 rounded-lg p-3 border border-zinc-800">
            <p className="text-xs text-zinc-500 font-mono mb-1">Target</p>
            <p className="text-sm text-zinc-300 font-medium">C99 / C11</p>
            <div className="mt-2 h-1 w-full bg-zinc-800 rounded-full overflow-hidden">
              <div className="h-full bg-orange-500 w-1/3"></div>
            </div>
          </div>
        </div>
      </aside>

      {/* Main Content Area */}
      <main className="flex-1 flex flex-col h-screen overflow-hidden bg-[#050505]">
        
        {/* Topbar */}
        <header className="h-16 border-b border-zinc-800 flex items-center px-8 shrink-0 bg-[#0a0a0a]/50 backdrop-blur-md">
          <h2 className="text-lg font-semibold text-white flex items-center gap-2">
            {currentData.icon}
            {currentData.title}
          </h2>
        </header>

        {/* Scrollable Content */}
        <div className="flex-1 overflow-y-auto p-8">
          <div className="max-w-4xl mx-auto">
            
            <AnimatePresence mode="wait">
              <motion.div
                key={currentData.id}
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -10 }}
                transition={{ duration: 0.2 }}
                className="space-y-12"
              >
                {/* Documentation Section */}
                <section>
                  <h1 className="text-3xl font-bold text-white mb-2">{currentData.title}</h1>
                  <p className="text-lg text-zinc-400 mb-8 pb-8 border-b border-zinc-800">
                    {currentData.description}
                  </p>
                  <div className="prose prose-invert prose-orange max-w-none">
                    {currentData.content}
                  </div>
                </section>

                {/* Code Section */}
                {currentData.code && (
                  <section className="mt-12">
                    <div className="flex items-center justify-between mb-4">
                      <h3 className="text-xl font-semibold text-white flex items-center gap-2">
                        <FileCode2 className="w-5 h-5 text-orange-500" />
                        Implementation
                      </h3>
                      <div className="bg-zinc-900 border border-zinc-800 px-3 py-1 rounded-md flex items-center gap-2">
                        <span className="w-2 h-2 rounded-full bg-orange-500"></span>
                        <span className="text-xs font-mono text-zinc-400">{currentData.code.filename}</span>
                      </div>
                    </div>
                    
                    <div className="rounded-xl overflow-hidden border border-zinc-800 shadow-2xl">
                      <SyntaxHighlighter
                        language={currentData.code.language}
                        style={vscDarkPlus}
                        customStyle={{
                          margin: 0,
                          padding: '1.5rem',
                          background: '#0a0a0a',
                          fontSize: '0.875rem',
                          lineHeight: '1.6',
                        }}
                        showLineNumbers={true}
                        lineNumberStyle={{
                          minWidth: '3em',
                          paddingRight: '1em',
                          color: '#404040',
                          textAlign: 'right',
                        }}
                      >
                        {currentData.code.snippet}
                      </SyntaxHighlighter>
                    </div>
                  </section>
                )}
              </motion.div>
            </AnimatePresence>

          </div>
        </div>
      </main>
    </div>
  );
}


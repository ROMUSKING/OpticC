import { motion } from 'motion/react';
import { Terminal, Cpu, Database, Network, HardDrive, CheckCircle2, Users, Clock, Code2 } from 'lucide-react';

const techStack = ['Rust (2021)', 'memmap2', 'redb (KV)', 'inkwell (LLVM 17+)', 'fuser'];

const phases = [
  {
    number: '01',
    title: 'Core Graph Infrastructure',
    weeks: 'Weeks 1-3',
    goal: 'Establish the zero-serialization mmap arena and KV store mapping.',
    tasks: [
      { name: 'The mmap Arena Allocator', desc: 'Append-only memory-mapped file (.optic/c_arena.bin) using memmap2.' },
      { name: 'Embedded KV-Store Integration', desc: 'Embed redb for metadata and deduplication tracking.' }
    ],
    icon: <Database className="w-5 h-5" />
  },
  {
    number: '02',
    title: 'C-Ingestion & The Preprocessor',
    weeks: 'Weeks 4-6',
    goal: 'Parse C99 into the graph, neutralizing #include overhead and expanding macros.',
    tasks: [
      { name: 'Lexer & Header Deduplication', desc: 'Hash file + state. Query redb. Parse and store on miss.' },
      { name: 'Dual-Node Macro Expansion', desc: 'Allocate Node A (Invocation) and Node B (Expanded AST).' },
      { name: 'C99 Recursive Descent Parser', desc: 'Parse constructs into C_AstNode topological edges.' }
    ],
    icon: <Code2 className="w-5 h-5" />
  },
  {
    number: '03',
    title: 'Graph-Based Static Analysis',
    weeks: 'Weeks 7-9',
    goal: 'Leverage whole-program graph visibility for instant alias analysis and taint tracking.',
    tasks: [
      { name: 'Grade Promotion Pass (Auto-noalias)', desc: 'DFS pointer provenance tracing for restrict/noalias tagging.' },
      { name: 'Taint Tracking & Lifetime Analysis', desc: 'Identify malloc/free nodes and Use-After-Free vulnerabilities.' }
    ],
    icon: <Network className="w-5 h-5" />
  },
  {
    number: '04',
    title: 'LLVM Backend Lowering',
    weeks: 'Weeks 10-12',
    goal: 'Translate the verified C-Graph into LLVM IR and emit binaries.',
    tasks: [
      { name: 'ABI-Compliant Struct Lowering', desc: 'Emit LLVM types matching target C ABI (System V, Win x64).' },
      { name: 'Control Flow & Basic Blocks', desc: 'Translate graph nodes into LLVM Basic Blocks via inkwell.' },
      { name: 'Applying Vectorization Hints', desc: 'Apply LLVM noalias attributes for SIMD auto-vectorization.' }
    ],
    icon: <Cpu className="w-5 h-5" />
  },
  {
    number: '05',
    title: 'VFS Projectional Tooling',
    weeks: 'Weeks 13-15',
    goal: 'Project the graph and its diagnostics out to the IDE via a FUSE filesystem.',
    tasks: [
      { name: 'The FUSE Driver (fuser)', desc: 'Map .optic/vfs/src/ to original C file paths.' },
      { name: 'Shadow Comments & Diagnostics', desc: 'Inject // [OPTIC ERROR] comments above offending AST nodes.' },
      { name: 'Expanded Macro Projection', desc: 'Expose fully evaluated macros in .optic/vfs/expanded_macros/.' }
    ],
    icon: <HardDrive className="w-5 h-5" />
  }
];

const teams = [
  { name: 'Squad A', role: 'Graph Infrastructure', desc: 'mmap arena, NodeOffset safety, redb integration' },
  { name: 'Squad B', role: 'C-Frontend', desc: 'Lexer, Parser, Macro expansion, #include deduplication' },
  { name: 'Squad C', role: 'Analysis & LLVM', desc: 'DFS/BFS alias analysis, Grade Promotion, inkwell lowering' },
  { name: 'Squad D', role: 'VFS & Tooling', desc: 'fuser projection, shadow comments, diagnostic directories' }
];

export default function App() {
  return (
    <div className="min-h-screen bg-[#0a0a0a] text-zinc-300 font-sans selection:bg-orange-500/30">
      {/* Header / Hero */}
      <header className="border-b border-zinc-800 bg-[#0f0f11] relative overflow-hidden">
        <div className="absolute inset-0 bg-[url('https://grainy-gradients.vercel.app/noise.svg')] opacity-20 mix-blend-overlay pointer-events-none"></div>
        <div className="absolute top-0 left-0 w-full h-1 bg-gradient-to-r from-orange-600 via-orange-500 to-amber-500"></div>
        
        <div className="max-w-7xl mx-auto px-6 py-16 relative z-10">
          <motion.div 
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.5 }}
            className="flex items-center gap-3 mb-6"
          >
            <Terminal className="w-8 h-8 text-orange-500" />
            <span className="font-mono text-orange-500 tracking-widest uppercase text-sm font-semibold">Project OCF</span>
          </motion.div>
          
          <motion.h1 
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.5, delay: 0.1 }}
            className="text-5xl md:text-7xl font-bold text-white tracking-tight mb-6"
          >
            Optic C-Frontend
          </motion.h1>
          
          <motion.p 
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.5, delay: 0.2 }}
            className="text-xl text-zinc-400 max-w-3xl leading-relaxed mb-10"
          >
            Legacy C99/C11 Compilation via mmap Graph Arena & VFS. Eliminating the #include I/O bottleneck and performing instant whole-program Link-Time Optimization.
          </motion.p>

          <motion.div 
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.5, delay: 0.3 }}
            className="flex flex-wrap gap-4 items-center"
          >
            <div className="flex items-center gap-2 bg-zinc-900 border border-zinc-800 px-4 py-2 rounded-full">
              <Clock className="w-4 h-4 text-zinc-500" />
              <span className="font-mono text-sm">15 Weeks ETA</span>
            </div>
            <div className="h-4 w-px bg-zinc-800 hidden sm:block"></div>
            <div className="flex flex-wrap gap-2">
              {techStack.map((tech) => (
                <span key={tech} className="bg-orange-500/10 text-orange-400 border border-orange-500/20 px-3 py-1 rounded-full text-xs font-mono uppercase tracking-wider">
                  {tech}
                </span>
              ))}
            </div>
          </motion.div>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-6 py-16 grid grid-cols-1 lg:grid-cols-12 gap-12">
        
        {/* Left Column: Phases */}
        <div className="lg:col-span-8 space-y-12">
          <div>
            <h2 className="text-2xl font-semibold text-white mb-8 flex items-center gap-3">
              <span className="w-8 h-px bg-orange-500"></span>
              Implementation Phases
            </h2>
            
            <div className="space-y-6">
              {phases.map((phase, idx) => (
                <motion.div 
                  key={phase.number}
                  initial={{ opacity: 0, x: -20 }}
                  whileInView={{ opacity: 1, x: 0 }}
                  viewport={{ once: true }}
                  transition={{ duration: 0.5, delay: idx * 0.1 }}
                  className="group relative bg-[#121214] border border-zinc-800 hover:border-orange-500/50 rounded-xl p-6 md:p-8 transition-colors"
                >
                  <div className="absolute top-0 right-0 p-6 opacity-10 group-hover:opacity-20 transition-opacity">
                    {phase.icon}
                  </div>
                  
                  <div className="flex flex-col md:flex-row md:items-baseline gap-4 mb-4">
                    <span className="font-mono text-4xl font-bold text-zinc-800 group-hover:text-orange-500/20 transition-colors">
                      {phase.number}
                    </span>
                    <h3 className="text-xl font-semibold text-white">{phase.title}</h3>
                    <span className="font-mono text-xs text-orange-400 bg-orange-400/10 px-2 py-1 rounded">
                      {phase.weeks}
                    </span>
                  </div>
                  
                  <p className="text-zinc-400 mb-6 font-medium">
                    Goal: {phase.goal}
                  </p>
                  
                  <div className="space-y-4">
                    {phase.tasks.map((task, tIdx) => (
                      <div key={tIdx} className="flex gap-4 items-start">
                        <div className="mt-1.5 w-1.5 h-1.5 rounded-full bg-orange-500 shrink-0"></div>
                        <div>
                          <h4 className="text-sm font-semibold text-zinc-200">{task.name}</h4>
                          <p className="text-sm text-zinc-500 mt-1">{task.desc}</p>
                        </div>
                      </div>
                    ))}
                  </div>
                </motion.div>
              ))}
            </div>
          </div>
        </div>

        {/* Right Column: Sidebar */}
        <div className="lg:col-span-4 space-y-8">
          
          {/* Definition of Done */}
          <motion.div 
            initial={{ opacity: 0, y: 20 }}
            whileInView={{ opacity: 1, y: 0 }}
            viewport={{ once: true }}
            className="bg-orange-500/5 border border-orange-500/20 rounded-xl p-6 relative overflow-hidden"
          >
            <div className="absolute top-0 left-0 w-1 h-full bg-orange-500"></div>
            <h3 className="text-lg font-semibold text-white mb-4 flex items-center gap-2">
              <CheckCircle2 className="w-5 h-5 text-orange-500" />
              Definition of Done
            </h3>
            <p className="text-sm text-zinc-400 leading-relaxed">
              Phase 0 is considered complete when the Optic C-Compiler can successfully ingest, analyze, project, and compile the <strong className="text-zinc-200">SQLite Amalgamation (sqlite3.c - ~250k LOC)</strong> into a working, test-passing shared library, while generating at least one "Taint Tracking" shadow comment via the VFS.
            </p>
          </motion.div>

          {/* Team Topology */}
          <motion.div 
            initial={{ opacity: 0, y: 20 }}
            whileInView={{ opacity: 1, y: 0 }}
            viewport={{ once: true }}
            className="bg-[#121214] border border-zinc-800 rounded-xl p-6"
          >
            <h3 className="text-lg font-semibold text-white mb-6 flex items-center gap-2">
              <Users className="w-5 h-5 text-zinc-400" />
              Team Topology
            </h3>
            <div className="space-y-6">
              {teams.map((team, idx) => (
                <div key={idx} className="relative pl-4 border-l border-zinc-800">
                  <div className="absolute -left-[5px] top-1.5 w-2 h-2 rounded-full bg-zinc-700"></div>
                  <div className="flex items-baseline justify-between gap-2 mb-1">
                    <h4 className="font-mono text-sm font-bold text-zinc-200">{team.name}</h4>
                    <span className="text-xs font-medium text-orange-400">{team.role}</span>
                  </div>
                  <p className="text-xs text-zinc-500 leading-relaxed">{team.desc}</p>
                </div>
              ))}
            </div>
          </motion.div>

        </div>
      </main>
    </div>
  );
}

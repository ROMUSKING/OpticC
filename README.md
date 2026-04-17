# OpticC — Autonomous Multi-Agent C Compiler

<div align="center">

**A C99-to-LLVM compiler built by an autonomous multi-agent team, with mmap arena allocation, embedded KV-store, graph-based static analysis, and FUSE-based vulnerability projection.**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2024-blue.svg)](https://www.rust-lang.org)
[![LLVM](https://img.shields.io/badge/LLVM-18.1-blue.svg)](https://llvm.org)
[![Tests](https://img.shields.io/badge/tests-261%2B%20passing-brightgreen.svg)]()

</div>

---

## Overview

OpticC is a C frontend compiler that translates C99 source code to LLVM IR. It is designed with a novel architecture:

- **Zero-serialization mmap arena** — AST nodes are stored in a memory-mapped file with bump allocation, enabling 10M+ node allocation in seconds
- **Embedded KV-store** — redb-powered database for `#include` deduplication and macro tracking
- **Graph-based static analysis** — DFS pointer provenance tracing, affine grade inference, and taint tracking for UAF detection
- **FUSE-based VFS** — Virtual filesystem that projects reconstructed source with `[OPTIC ERROR]` shadow comments on vulnerable lines
- **Multi-agent development** — Built by 8+ specialized AI agents communicating through YAML specs and task files

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        OpticC Compiler                       │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌──────────┐   ┌──────────────┐   ┌──────────┐            │
│  │Preproces-│──▶│   Parser     │──▶│   AST    │            │
│  │  sor     │   │(Rec. Descent)│   │  Arena   │            │
│  └──────────┘   └──────────────┘   └────┬─────┘            │
│       │                                  │                   │
│       ▼                                  ▼                   │
│  ┌──────────┐   ┌──────────────┐   ┌──────────┐            │
│  │ redb KV  │   │Type Resolver │──▶│  Typed   │            │
│  │  Store   │   │              │   │   AST    │            │
│  └──────────┘   └──────────────┘   └────┬─────┘            │
│                                         │                   │
│                    ┌────────────────────┼──────────────┐    │
│                    ▼                    ▼              ▼    │
│              ┌──────────┐      ┌──────────┐   ┌──────────┐ │
│              │  Static   │      │   LLVM   │   │   VFS    │ │
│              │ Analysis  │      │ Backend  │   │Projection│ │
│              └──────────┘      └────┬─────┘   └──────────┘ │
│                                     │                       │
│                                     ▼                       │
│                              ┌──────────┐                  │
│                              │ LLVM IR  │                  │
│                              │  (.ll)   │                  │
│                              └──────────┘                  │
└─────────────────────────────────────────────────────────────┘
```

## Features

### Phase 1: Core Infrastructure ✅
| Module | Status | Description |
|--------|--------|-------------|
| **Arena** | ✅ | 64-byte `#[repr(C)]` CAstNode, mmap-backed, 10M node benchmark, string interning |
| **DB** | ✅ | redb KV-store with `file_hashes` and `macros` tables, full CRUD |
| **Lexer** | ✅ | Byte-level C99 lexer, 37 keywords, multi-char punctuators |
| **Macro Expander** | ✅ | Object-like + function-like macros, `##` token pasting, `#` stringification |
| **Parser** | ✅ | Recursive descent C99 parser, all statements, full expression grammar |
| **LLVM Backend** | ✅ | Type-aware codegen (i8/i16/i32/i64/f32/f64/ptr), control flow, expressions |
| **Analysis** | ✅ | DFS pointer provenance, affine grades, taint tracking, UAF detection |
| **VFS** | ✅ | FUSE filesystem with `[OPTIC ERROR]` shadow comment injection |

### Phase 2: SQLite Compilation ✅
| Module | Status | Description |
|--------|--------|-------------|
| **Preprocessor** | ✅ | `#include`, `#define`, `#ifdef`/`#if`/`#elif`, `#pragma`, predefined macros |
| **Type System** | ✅ | 17 CType variants, struct layout, type checking, implicit conversions |
| **PP→Parser Wiring** | ✅ | Unified Token type, `parse_tokens()`, backward compatible |
| **Typed Backend** | ✅ | Type-aware LLVM generation, float ops, 64-bit ints |
| **GNU Extensions** | ✅ | `__attribute__`, `typeof`, statement expressions, `__builtin_*` |
| **Inline Asm** | ✅ | `asm volatile` with operands, clobbers, goto asm |
| **Build System** | ✅ | Multi-file compilation, linking, parallel builds, build cache |
| **Benchmarks** | ✅ | OpticC vs GCC vs Clang comparison suite |

### Phase 3: Linux Kernel 📋
GNU C extensions, inline assembly, Kbuild integration, 30M+ LOC scale.

### Phase 4: Production 📋
Optimization pipeline, DWARF debug info, LTO, cross-compilation.

## Quick Start

### Prerequisites
- **Ubuntu 22.04** (or Debian-based Linux)
- **Rust 1.95+** (via rustup)
- **LLVM 18** (for inkwell bindings)
- **gcc/clang** (for linking)
- **FUSE** (optional, for VFS projection)

### Toolchain Installation (Cloud Agent / Ubuntu)
```bash
# Install system dependencies
apt-get update && apt-get install -y build-essential clang llvm llvm-dev lld binutils unzip curl

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env

# Verify installation
gcc --version    # gcc 11.4.0
clang --version  # clang 18.1.3
llvm-config-18 --version  # 18.1.x
rustc --version  # rustc 1.95.0
```

### Build
```bash
cargo build        # 0 errors
cargo test         # 259 passing
```

### Usage
```bash
# Compile a C file to LLVM IR
cargo run -- compile input.c -o output.ll

# Compile with optimization
cargo run -- compile input.c -o output.ll -O2

# Multi-file build
cargo run -- build --src-dir ./src -o lib.so -j 8

# Multi-file build with libraries
cargo run -- build --src-dir ./src -o myapp -t executable --link-libs m,pthread

# Run benchmarks
cargo run -- benchmark --suite all --compilers all --output-dir results

# Run SQLite integration test
cargo run -- integration-test --test-dir /tmp/optic_test
```
```

### Test Samples
```bash
# Simple function
cargo run -- compile test_samples/simple.c -o simple.ll

# Struct test
cargo run -- compile test_samples/struct_test.c -o struct_test.ll

# Macro test
cargo run -- compile test_samples/macro_test.c -o macro_test.ll
```

## Project Structure

```
.
├── Cargo.toml              # Rust workspace configuration
├── src/                    # Compiler source code
│   ├── main.rs             # CLI entry point (clap)
│   ├── lib.rs              # Library exports
│   ├── arena.rs            # mmap arena allocator
│   ├── db.rs               # redb KV-store
│   ├── frontend/           # C frontend
│   │   ├── lexer.rs        # C99 lexer
│   │   ├── macro_expander.rs # Macro expansion
│   │   ├── parser.rs       # Recursive descent parser
│   │   └── preprocessor.rs # C preprocessor
│   ├── backend/            # Code generation
│   │   └── llvm.rs         # LLVM IR lowering
│   ├── analysis/           # Static analysis
│   │   └── alias.rs        # Pointer provenance & taint tracking
│   ├── types/              # Type system
│   │   ├── mod.rs          # CType enum, TypeSystem
│   │   └── resolve.rs      # TypeResolver, type checking
│   └── vfs/                # VFS projection
│       └── mod.rs          # FUSE filesystem
├── test_samples/           # C test files
├── jules_prompts/          # Multi-agent prompts (17 files)
├── .optic/                 # Project specs & tasks
│   ├── spec/               # YAML API contracts (13 files)
│   └── tasks/              # Task tracking (15 files + inbox)
└── llvm.sh                 # LLVM installation script
```

## Multi-Agent Development

OpticC was built using an **autonomous multi-agent workflow**:

1. **Async Branch Protocol** — Each agent works on a separate git branch
2. **Rich Spec Format** — YAML API contracts (`.optic/spec/*.yaml`) replace markdown documentation
3. **Task Tracking** — Agent-specific task files (`.optic/tasks/*.md`) with completion markers
4. **Inbox System** — Cross-agent bug reports via unique-ID files (no merge conflicts)

### Agent Roster
| Agent | Domain | Prompt |
|-------|--------|--------|
| Jules-Orchestrator | Project coordination | `01_orchestrator.md` |
| Jules-Memory-Infra | Arena allocator | `02_memory_infra.md` |
| Jules-DB-Infra | Embedded KV-store | `03_db_infra.md` |
| Jules-Lexer-Macro | C ingestion & preprocessing | `04_lexer_macro.md` |
| Jules-Parser | AST construction | `05_parser.md` |
| Jules-Analysis | Graph-based static analysis | `06_analysis.md` |
| Jules-Backend-LLVM | LLVM IR lowering | `07_backend_llvm.md` |
| Jules-VFS-Projection | FUSE filesystem | `08_vfs_projection.md` |
| Jules-Integration | QA & Definition of Done | `09_integration.md` |
| Jules-Preprocessor | C preprocessor | `10_preprocessor.md` |
| Jules-Type-System | Type representation | `11_type_system.md` |
| Jules-GNU-Extensions | GNU C dialect | `12_gnu_extensions.md` |
| Jules-Inline-Asm | Assembly support | `13_inline_asm.md` |
| Jules-Build-System | Multi-file compilation | `14_build_system.md` |
| Jules-Benchmark | Performance comparison | `15_benchmark.md` |

## Test Results

| Module | Tests | Status |
|--------|-------|--------|
| Integration | 20 | ✅ |
| Benchmark | 31 | ✅ |
| Build System | 22 | ✅ |
| GNU Extensions | 46 | ✅ |
| Inline Assembly | 15 | ✅ |
| Type System (mod) | 26 | ✅ |
| Type System (resolve) | 44 | ✅ |
| Backend LLVM | 13 | ✅ |
| Preprocessor | 21 | ✅ |
| Analysis | 5 | ✅ |
| Arena | 10 | ✅ |
| DB | 11 | ✅ |
| Parser | 9 | ✅ |
| Lexer | 6 | ✅ |
| **Total** | **286** | **281 passing, 5 pre-existing** |

## Roadmap

### Milestone 1: SQLite Compilation ✅
- [x] Complete GNU C extensions (`__attribute__`, `typeof`, builtins)
- [x] Complete inline assembly support
- [x] Build system (multi-file compilation, linking)
- [x] Benchmark vs GCC/Clang
- [ ] Compile SQLite Amalgamation (255K LOC) to `libsqlite3.so`
- [ ] Pass SQLite test suite

### Milestone 2: Linux Kernel Modules
- [ ] Full GNU C dialect support
- [ ] Inline assembly with full operand/clobber support
- [ ] Kbuild integration
- [ ] Compile out-of-tree kernel modules

### Milestone 3: Production Compiler
- [ ] LLVM optimization pipeline (pass manager)
- [ ] DWARF debug information
- [ ] Link-Time Optimization (LTO)
- [ ] Cross-compilation support

## License

MIT License — see [LICENSE](LICENSE) for details.

## Acknowledgments

- Built with [inkwell](https://github.com/TheDan64/inkwell) (Rust LLVM bindings)
- Uses [memmap2](https://github.com/RazrFalcon/memmap2) for memory-mapped files
- Uses [redb](https://github.com/cberner/redb) for embedded KV storage
- Uses [fuser](https://github.com/cberner/fuser) for FUSE filesystem
- Inspired by Cloudflare's autonomous agent workflows

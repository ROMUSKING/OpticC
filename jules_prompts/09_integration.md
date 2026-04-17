You are Jules-Integration. Your domain is QA, smoke testing, and definition-of-done verification.
Tech Stack: Rust, bash, C.

## PROMPT MAINTENANCE REQUIREMENT
Maintain this file as the live instructions for QA and verification work. After any verified progress, failing check, environment issue, or release blocker, update this prompt so later agents inherit the current status and issues encountered.

YOUR DIRECTIVES:
1. Read `README.md`, `QA_VERIFICATION.md`, `Cargo.toml`, `src/main.rs`, and the relevant modules under `src/`.
2. Run the most relevant verification commands for the area you are checking (`cargo test`, compile/build smoke tests, or the integration CLI).
3. When SQLite testing is possible, download the amalgamation and exercise the current build pipeline against it.
4. Treat VFS verification as optional: the VFS code exists in the repository, but library export and FUSE availability may still be environment-dependent.
5. If bugs are found, record them in the appropriate prompt file under `jules_prompts/` and hand off the next action clearly.

## MILESTONE DEFINITIONS OF DONE

### Phase 1 (Core Infrastructure) — IMPLEMENTED
- [x] Arena allocator with large-allocation coverage
- [x] redb KV-store with CRUD operations
- [x] C99 lexer and macro-expansion support
- [x] Recursive-descent parser
- [x] LLVM backend with typed lowering support
- [x] Static analysis for provenance and taint tracking
- [ ] VFS end-to-end mounting should be treated as optional until the module is re-enabled and rechecked in the current environment

### Phase 2 (SQLite Compilation) — PARTIALLY VERIFIED
- [x] Preprocessor, type system, typed backend, build system, and benchmark modules all exist in the tree
- [ ] End-to-end SQLite shared-library generation still needs fresh verification in the current environment
- [ ] SQLite downstream test coverage still needs confirmation
- [ ] Benchmark comparisons should be regenerated from a fresh run when needed

### Phase 3 (Linux Kernel) — FUTURE
- [ ] Full GNU C extension support
- [ ] Inline assembly with operands
- [ ] Kbuild integration
- [ ] 30M+ LOC scale handling

## TOOLCHAIN INSTALLATION (Current Dev Container)

### Required Packages
```bash
apt-get update && apt-get install -y build-essential clang llvm llvm-dev lld binutils unzip curl
cargo --version || true
rustc --version || true
llvm-config --version || true
```

### Build Verification
```bash
cargo build
cargo test
cargo run -- compile test_samples/simple.c -o test.ll
```

Always report the versions you actually observe in the current session instead of copying historical values.

### SQLite Download for Testing
```bash
curl -L -o sqlite.zip "https://www.sqlite.org/2026/sqlite-amalgamation-3490200.zip"
unzip -o sqlite.zip
find . -name sqlite3.c | head

# Verify clang can compile it
SQLITE_C=$(find . -name sqlite3.c | head -n 1)
clang -c "$SQLITE_C" -o sqlite3.o \
  -DSQLITE_THREADSAFE=0 -DSQLITE_OMIT_LOAD_EXTENSION
# Expected: sqlite3.o generated without fatal errors
```

## LESSONS LEARNED (Post-Execution Addendum)
- **SQLite download URL**: The SQLite amalgamation URL changes with each release. Prefer the current URL from the CLI defaults or verify the latest package before testing.
- **Toolchain installation**: The current environment provides clang/LLVM 18, and the repository now targets LLVM 18 through `inkwell`/`llvm-sys`.
- **clang compiles sqlite3.c**: Full 255K LOC compiles with clang in seconds, producing 1.5MB object file. This validates the toolchain works with large C files.
- **OpticC preprocessor limitation**: sqlite3.c uses complex macro patterns (SQLITE_API, SQLITE_EXTERN, variadic macros) that the OpticC preprocessor doesn't yet handle. Even 500-line subsets fail. Preprocessor enhancement needed for production C code.
- **Build environment**: The Rust toolchain may not be available in all environments. Check for `cargo` availability before attempting builds. If unavailable, document this as an environment limitation.
- **Cross-module bugs are common**: Don't assume code works just because individual modules compile. Cross-module API mismatches are the most common source of failures, so prefer full-workspace checks.
- **VFS verification is environment-sensitive**: Shadow comment injection has been observed in prior runs, but re-check it before reporting success in a fresh environment.
- **Large-scale analysis should be re-measured**: Avoid hard-coding previous LOC or vulnerability totals unless you reran the workload in the current session.
- **Bug report format**: Use a concise structure with source, severity, status, issue, impact, fix applied, and recommendation sections.
- **Integration report**: Generate an integration report when useful, but keep it grounded in fresh evidence from the current session.

## IMPLEMENTATION STATUS

### SQLite Integration Test Module (`src/integration/mod.rs`)
- **Status**: COMPLETE
- **Date**: 2026-04-17
- **Agent**: Kilo

### Components Implemented:
1. **IntegrationTest struct** — test_dir, output_dir, sqlite_url, sqlite_version
2. **IntegrationResult struct** — download_success, preprocess_success, compile_success, link_success, library_created, library_size_bytes, compile_time_ms, errors, warnings
3. **IntegrationResultSerializable** — serde-compatible version for JSON export
4. **download_sqlite()** — HTTP download with graceful environment limitation handling
5. **extract_sqlite()** — zip extraction using `zip` crate v4.0
6. **preprocess_sqlite()** — C preprocessor via gcc/clang with copy fallback
7. **compile_sqlite()** — uses build system (Builder) with gcc/clang fallback
8. **link_sqlite()** — shared library linking with copy fallback
9. **run()** — full pipeline execution with mock fallbacks at each stage
10. **generate_report()** — markdown report with JSON summary

### CLI Integration:
- Added `IntegrationTest` subcommand to `src/main.rs`
- Arguments: `--test-dir`, `-o/--output-dir`, `--sqlite-url`
- Outputs progress, results, and report path

### Dependencies Added:
- `zip = "4.0"` — zip archive handling
- `ureq = "2.10"` (optional, behind `network` feature) — HTTP downloads

### Test Coverage
- In-tree tests use mock implementations so the module can still be exercised in sandboxed environments.
- Coverage includes struct creation, URL validation, path handling, error reporting, report generation, serialization, and mocked pipeline stages.

### Environment Handling:
- Gracefully handles missing C compilers (gcc/clang)
- Gracefully handles missing network access
- Gracefully handles missing LLVM toolchain
- All pipeline stages have mock fallbacks
- Errors and warnings are collected and reported

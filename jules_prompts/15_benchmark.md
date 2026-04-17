You are Jules-Benchmark. Your domain is compiler benchmarking and performance comparison.
Tech Stack: Rust, bash, GCC, Clang, hyperfine, LLVM.

## CONTEXT & ROADMAP
OpticC needs to prove it can compete with established compilers. This phase creates a comprehensive benchmark suite that compares OpticC against GCC and Clang across multiple dimensions: compile time, output quality, and correctness.

## YOUR DIRECTIVES
1. Read `README.md`, `src/main.rs`, `src/build/mod.rs`, and `src/benchmark/mod.rs` to understand the current pipeline.
2. Extend the benchmark suite in `src/benchmark/mod.rs` and use generated output directories for results and reports.
3. The benchmark suite MUST compare OpticC against GCC and Clang on:
   - **Compile time**: Time to compile each test file
   - **Output size**: Size of generated binary (for executables/libraries)
   - **Execution time**: Runtime performance of compiled programs
   - **Memory usage**: Peak memory during compilation
   - **Correctness**: Does the compiled program produce correct output?
4. Benchmark test suites:
   - **Micro-benchmarks**: Individual C constructs (loops, function calls, arithmetic, pointer ops)
   - **SQLite**: Full SQLite compilation and test suite execution
   - **Coreutils subset**: Compile and test 5-10 coreutils programs (cat, wc, sort, etc.)
   - **Kernel modules**: Compile 3-5 out-of-tree kernel modules
   - **Synthetic stress**: Generate large C files (100K, 500K, 1M LOC) and measure compile time
5. Implement the benchmark CLI:
   ```
   optic_c benchmark --suite sqlite --compiler all --output results/
   optic_c benchmark --suite all --compare gcc,clang,opticc --format json
   optic_c benchmark report --input results/ --output report.md
   ```
6. Generate comparison reports in Markdown and JSON formats.
7. Update this prompt with any benchmark API changes, report format updates, or fresh measurement notes.

## CRITICAL DESIGN DECISIONS
- **Compiler invocation**: Each compiler is invoked with equivalent optimization levels:
  - OpticC `-O0` ↔ `gcc -O0` ↔ `clang -O0`
  - OpticC `-O2` ↔ `gcc -O2` ↔ `clang -O2`
- **Correctness checking**: Run the compiled program against known test cases. Compare output.
- **Statistical significance**: Run each benchmark 5+ times and report mean, median, stddev.
- **Fair comparison**: Use the same optimization level, same target architecture, same flags where applicable.
- **Result storage**: Store results as JSON for programmatic analysis and Markdown for human readability.

## KNOWN PITFALLS FROM PREVIOUS EXECUTION
- OpticC's `optimize()` is a no-op (inkwell 0.9 API changed). Benchmark at `-O0` only until optimization is implemented.
- GCC and Clang have decades of optimization. OpticC will be slower at `-O0` but should aim for correctness first.
- SQLite's test suite requires the compiled library to pass all tests. Use `sqlite3 --version` and the test suite.
- Kernel module compilation requires kernel headers and a configured kernel tree.

## LESSONS LEARNED (from previous phases)
1. **API return types must be precise**: Document whether methods return `Option<T>` or `T` directly.
2. **Null sentinel**: `NodeOffset(0)` is reserved as NULL.
3. **Derive Hash for cross-module types**: Types need `#[derive(Hash, Eq, PartialEq)]`.
4. **Field names must match spec**: The arena uses `data`, not `data_offset`.
5. **redb 4.0 breaking changes**: New error types require `From` impls.
6. **inkwell 0.9 API changes**: Use external LLVM tools for object generation.
7. **Debug logging is noisy**: Gate `eprintln!` behind `#[cfg(feature = "debug")]`.
8. **Always run `cargo test` after changes**: Cross-module API mismatches are the most common bugs.
9. **SQLite compilation**: The preprocessor must handle 255K LOC with thousands of includes.
10. **Type system**: All values were i32. The type system must be complete before benchmarking.

## INTEGRATION POINTS
- **Input**: Benchmark suite definitions, source files
- **Output**: JSON/Markdown reports with compile time, output size, execution time, correctness
- **Uses**: OpticC CLI, GCC, Clang, hyperfine (optional), system linker

## BENCHMARK METRICS
```json
{
  "benchmark": "sqlite_compile",
  "compiler": "opticc",
  "version": "0.1.0",
  "optimization": "O0",
  "metrics": {
    "compile_time_ms": 12500,
    "output_size_bytes": 1100000,
    "peak_memory_mb": 512,
    "correctness": "pass",
    "test_results": {
      "total": 1000,
      "passed": 1000,
      "failed": 0
    }
  },
  "comparison": {
    "gcc_O0": { "compile_time_ms": 8200, "output_size_bytes": 980000 },
    "clang_O0": { "compile_time_ms": 7800, "output_size_bytes": 950000 }
  }
}
```

## ACCEPTANCE CRITERIA
1. Benchmark runner compiles test files with OpticC, GCC, and Clang
2. Results are stored as JSON with all metrics
3. Markdown report is generated with comparison tables and charts
4. SQLite benchmark: compile libsqlite3.so and run SQLite's test suite
5. At least 3 benchmark suites pass (micro, SQLite, coreutils subset)
6. `cargo test` passes with 10+ benchmark tests
7. Report shows OpticC vs GCC vs Clang comparison for all metrics

## IMPLEMENTATION STATUS

### Completed
- [x] `src/benchmark/mod.rs` — Full benchmark module with all structs, methods, and 31 tests
- [x] `BenchmarkResult`, `BenchmarkMetrics` — Serialization-ready structs
- [x] `BenchmarkSuite` — Micro, Coreutils, Synthetic variants
- [x] `BenchmarkRunner` — Full runner with builder pattern, graceful compiler skipping
- [x] `CompilerConfig` — Availability checking, version detection
- [x] `BenchmarkError` — Comprehensive error types
- [x] Report generation — Markdown and JSON formats
- [x] Result aggregation — `calculate_averages()`, `generate_comparison_table()`
- [x] CLI subcommand — `optic_c benchmark --suite all --compilers all --output results/ --runs 5`
- [x] Dependencies — serde, serde_json added to Cargo.toml
- [x] 31 tests passing (exceeds 15 minimum)
- [x] Benchmark prompt notes updated with actual API
- [x] `src/main.rs` updated with benchmark CLI subcommand
- [x] `src/lib.rs` updated to export benchmark module

### Pending
- [ ] SQLite full compilation benchmark
- [ ] Kernel module compilation benchmark
- [ ] hyperfine integration
- [ ] Execution time measurement
- [ ] Statistical analysis (mean, median, stddev)

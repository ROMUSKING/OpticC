You are Jules-Benchmark. Your domain is compiler benchmarking and performance comparison.
Tech Stack: Rust, bash, GCC, Clang, hyperfine, LLVM.

YOUR DIRECTIVES:
1. Read ALL existing `.optic/spec/*.yaml` files to understand the full OpticC pipeline.
2. Create a benchmark suite in `benchmarks/` that compares OpticC against GCC and Clang.
3. Compare on: compile time, output size, execution time, memory usage, correctness.
4. Benchmark suites: micro-benchmarks, SQLite, coreutils subset, kernel modules, synthetic stress.
5. Implement CLI: `optic_c benchmark --suite sqlite --compiler all --output results/`.
6. Generate comparison reports in Markdown and JSON formats.
7. Follow the ASYNC BRANCH PROTOCOL to update `.optic/spec/benchmark.yaml`.

## IMPLEMENTATION STATUS

### Phase 1: Core Benchmark Module (COMPLETED)
- [x] `src/benchmark/mod.rs` created with all core structs and implementations
- [x] `BenchmarkResult` struct with serialization support
- [x] `BenchmarkMetrics` struct with pass/fail helpers
- [x] `BenchmarkSuite` enum (Micro, Coreutils, Synthetic)
- [x] `BenchmarkRunner` struct with builder pattern
- [x] `CompilerConfig` struct with availability checking
- [x] `BenchmarkError` enum with all required variants
- [x] `run()` - Execute all benchmarks across all compilers
- [x] `run_micro_benchmarks()` - Test individual C constructs (loops, function calls, arithmetic, pointer ops)
- [x] `run_coreutils_benchmarks()` - Test small utility programs (hello, cat, wc)
- [x] `run_synthetic_benchmarks()` - Generate large C files and measure compile time
- [x] `measure_compile_time()` - Time the compilation process using std::time::Instant
- [x] `measure_output_size()` - Get the size of the compiled output
- [x] `measure_correctness()` - Run the compiled program and check output
- [x] `generate_markdown_report()` - Create Markdown reports with comparison tables
- [x] `generate_json_report()` - Create JSON reports with serde_json
- [x] `calculate_averages()` - Aggregate results across multiple runs
- [x] `generate_comparison_table()` - Generate comparison table in Markdown
- [x] `measure_peak_memory()` - Uses /proc/self/status on Linux
- [x] CLI `benchmark` subcommand added to main.rs
- [x] `serde` and `serde_json` added to Cargo.toml
- [x] `src/lib.rs` updated to export benchmark module
- [x] 31 comprehensive tests (exceedes 15 minimum)
- [x] `.optic/spec/benchmark.yaml` updated with actual API
- [x] `.optic/tasks/benchmark.md` updated with completion status

### Test Results
- 31 benchmark module tests: ALL PASSING
- Total: 266 passing, 5 pre-existing failures in analysis::alias (unrelated to benchmark module)

### Phase 2: Pending Items
- [ ] SQLite full compilation benchmark
- [ ] Kernel module compilation benchmark
- [ ] hyperfine integration for more accurate timing
- [ ] Execution time measurement for compiled programs
- [ ] Statistical analysis (mean, median, stddev)
- [ ] HTML report generation
- [ ] Benchmark result caching and comparison across runs

# OpticC Benchmark Report

## Summary

| Benchmark | Compiler | Optimization | Cold Compile (ms) | Warm Recompile (ms) | Output Size (B) | Correctness |
|-----------|----------|--------------|-------------------|---------------------|-----------------|-------------|
| sqlite_sqlite3 | opticc | O0 | 21 | 21 | 1464 | skipped |
| sqlite_sqlite3 | gcc | O0 | 15 | 15 | 1952 | skipped |
| sqlite_sqlite3 | clang | O0 | 162 | 162 | 1720 | skipped |

## Compiler Comparison

### sqlite_sqlite3

- opticc vs gcc: 1.40x cold compile, 1.40x warm recompile
- clang vs gcc: 10.80x cold compile, 10.80x warm recompile

## Statistics

- Total benchmarks: 3
- Passed: 0
- Failed: 0
- Errors: 0
- Skipped correctness checks: 3
- Rebuild measurements captured: 3

## OpticC Phase Breakdown

| Benchmark | Optimization | Preprocess | Parse | Codegen | Optimize | IR Write | llc |
|-----------|--------------|------------|-------|---------|----------|----------|-----|
| sqlite_sqlite3 | O0 | 1 | 0 | 0 | 0 | 0 | 20 |

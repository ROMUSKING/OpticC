# OpticC Benchmark Report

## Summary

| Benchmark | Compiler | Optimization | Cold Compile (ms) | Warm Recompile (ms) | Output Size (B) | Correctness |
|-----------|----------|--------------|-------------------|---------------------|-----------------|-------------|
| sqlite_sqlite3 | opticc | O0 | 19 | 19 | 1464 | skipped |

## Compiler Comparison

### sqlite_sqlite3


## Statistics

- Total benchmarks: 1
- Passed: 0
- Failed: 0
- Errors: 0
- Skipped correctness checks: 1
- Rebuild measurements captured: 1

## OpticC Phase Breakdown

| Benchmark | Optimization | Preprocess | Parse | Codegen | Optimize | IR Write | llc |
|-----------|--------------|------------|-------|---------|----------|----------|-----|
| sqlite_sqlite3 | O0 | 1 | 0 | 0 | 0 | 0 | 18 |

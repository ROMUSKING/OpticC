# OpticC Benchmark Report

## Summary

| Benchmark | Compiler | Optimization | Cold Compile (ms) | Warm Recompile (ms) | Output Size (B) | Correctness |
|-----------|----------|--------------|-------------------|---------------------|-----------------|-------------|
| sqlite_flags | opticc | O0 | 88 | 88 | 1992 | skipped |
| sqlite_varint | opticc | O0 | 70 | 70 | 1960 | skipped |
| sqlite_struct_state | opticc | O0 | 64 | 64 | 1888 | skipped |
| sqlite_arrow_cursor | opticc | O0 | 68 | 68 | 1856 | skipped |

## Compiler Comparison

### sqlite_arrow_cursor

### sqlite_varint

### sqlite_flags

### sqlite_struct_state


## Statistics

- Total benchmarks: 4
- Passed: 0
- Failed: 0
- Errors: 0
- Skipped correctness checks: 4
- Rebuild measurements captured: 4

## OpticC Phase Breakdown

| Benchmark | Optimization | Preprocess | Parse | Codegen | Optimize | IR Write | llc |
|-----------|--------------|------------|-------|---------|----------|----------|-----|
| sqlite_flags | O0 | 60 | 3 | 1 | 0 | 0 | 24 |
| sqlite_varint | O0 | 48 | 2 | 1 | 0 | 0 | 19 |
| sqlite_struct_state | O0 | 44 | 2 | 1 | 0 | 0 | 17 |
| sqlite_arrow_cursor | O0 | 47 | 2 | 1 | 0 | 0 | 18 |

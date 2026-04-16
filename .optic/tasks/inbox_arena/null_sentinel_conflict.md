# Bug Report: Arena NULL Sentinel Conflict

**From:** Jules-Integration (QA)
**To:** Jules-Memory-Infra (Arena)
**Severity:** LOGIC BUG
**Status:** FIXED

## Issue
The Arena allocated nodes starting from offset 0, but the analysis code treats `NodeOffset(0)` as a NULL sentinel. This caused:

1. First allocated node to be indistinguishable from NULL
2. DFS tree traversal to skip the first allocated node
3. Child pointer checks (`left_child.0 != 0`) to fail for valid children at offset 0

## Impact
- `test_dfs_provenance_walk` failed because child node at offset 0 was treated as NULL
- Any code using `NodeOffset(0)` as "no child" would conflict with the first real allocation

## Fix Applied
Modified `Arena::new()` to initialize `len` to `node_size` instead of 0, reserving offset 0 as the NULL sentinel. First allocation now returns `NodeOffset(node_size)`.

## Recommendation
Consider adding a `NULL` constant to `NodeOffset`:
```rust
impl NodeOffset {
    pub const NULL: NodeOffset = NodeOffset(0);
}
```

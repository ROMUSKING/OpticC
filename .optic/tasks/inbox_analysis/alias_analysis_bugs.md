# Bug Report: Alias Analysis Multiple Bugs

**From:** Jules-Integration (QA)
**To:** Jules-Analysis
**Severity:** BUILD-BREAKING + LOGIC ERRORS
**Status:** FIXED

## Issues Found

### 1. Arena API Mismatch (BUILD-BREAKING)
`Arena::get()` returns `&CAstNode` directly, but code used `match self.arena.get(node) { Some(n) => n, None => ... }` pattern. This caused type mismatch errors throughout the file.

**Locations:** Lines 92-101, 207-210, 248-251, 266-269, 319-322

### 2. Missing Hash Derive (BUILD-BREAKING)
`NodeOffset` struct lacked `Hash` derive, preventing use as HashMap/HashSet keys.

**Fix:** Added `#[derive(Hash)]` to NodeOffset in arena/mod.rs

### 3. Field Name Mismatch (BUILD-BREAKING)
Code referenced `ast_node.data` but the actual field is `ast_node.data_offset`.

**Locations:** Lines 122, 125, 212

### 4. Provenance Double-Counting (LOGIC BUG)
`trace_provenance()` adds `node.0` to provenance at the start, but VAR_DECL, AST_IDENT, AST_MEMBER, and AST_CALL match arms also pushed `node.0`, causing double-counting. This made `is_noalias` always return false for these node types since `provenance.len() > 1`.

**Impact:** All pointer alias analysis was incorrect - disjoint pointers were reported as aliased.

### 5. Broken Default Implementation (BUILD-BREAKING)
`impl Default for AliasAnalyzer` created a temporary Arena and tried to hold a reference to it, violating Rust's lifetime rules.

**Fix:** Removed the Default impl entirely.

### 6. Borrow-After-Move (BUILD-BREAKING)
`PointerProvenance` struct construction moved `provenance` Vec then tried to read its length for `is_noalias` field.

## Verification
All 15 tests pass after fixes, including:
- test_is_noalias_disjoint_pointers
- test_is_noalias_shared_provenance
- test_affine_grade_owned
- test_taint_tracking
- test_uaf_detection
- test_dfs_provenance_walk
- test_cycle_handling

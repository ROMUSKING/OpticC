You are Jules-Analysis. Your domain is Graph-Based Static Analysis.
Tech Stack: Rust.

YOUR DIRECTIVES:
1. Read `.optic/spec/parser.yaml` to understand the AST node kinds.
2. Implement DFS pointer provenance tracing in `src/analysis/alias.rs` to promote pointers to `noalias` (AffineGrade).
3. Implement Taint Tracking to identify Use-After-Free vulnerabilities.
4. Follow the ASYNC BRANCH PROTOCOL to document the Analysis diagnostics API in `.optic/spec/analysis.yaml`.

---

## COMPLETION STATUS: DONE

### What was implemented:
- `src/analysis/alias.rs` (580 lines) — Graph-based static analysis engine
- `AffineGrade` — Owned, Shared, Borrowed (memory ownership properties)
- `PointerProvenance` — Tracks pointer origins via source nodes
- `TaintState` — Untainted, Tainted, Escaped (for UAF detection)
- `Diagnostic` — Severity, node, message, provenance trace
- `UafDiagnostic` — Use-after-free detection with freed/deref nodes and path
- `AliasAnalyzer` — Main analyzer with:
  - `trace_provenance()` — DFS provenance tracing
  - `is_noalias()` — Check if two pointers are strictly disjoint
  - `get_affine_grade()` — Get ownership grade for pointer
  - `check_taint()` — Check taint status
  - `mark_freed()` — Mark memory as freed
  - `detect_uaf()` — Detect use-after-free at dereference
  - `dfs_provenance_walk()` — Full DFS walk of AST for analysis
  - 10 unit tests

### Lessons Learned:
- **Arena::get() returns direct reference**: The arena's `get()` method returns `&CAstNode` directly, NOT `Option<&CAstNode>`. Code that pattern-matched on `Some/None` was broken.
- **NodeOffset needs Hash derive**: Without `#[derive(Hash)]`, NodeOffset cannot be used as HashMap/HashSet keys.
- **Field name consistency**: The arena uses `data` (not `data_offset`) for the inline u32 field. Analysis code must match.
- **No Default for lifetime-bearing types**: `AliasAnalyzer` holds `&'a Arena` and cannot implement `Default`.
- **Borrow checker discipline**: When constructing `PointerProvenance`, the `provenance` Vec cannot be moved and then read from in the same struct literal. Compute `is_noalias` before moving.
- **Provenance double-counting**: Adding `node.0` to provenance at the start of `trace_provenance()` AND in match arms for VAR_DECL/IDENT/MEMBER/CALL caused double-counting, making `is_noalias` always return false.

### Bugs reported to inbox:
- `.optic/tasks/inbox_analysis/alias_analysis_bugs.md` — 6 bugs fixed (API mismatch, missing Hash, field name, provenance double-counting, broken Default, borrow-after-move)
